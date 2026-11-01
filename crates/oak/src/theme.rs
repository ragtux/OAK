//! The colours and fonts the shell paints from, loaded from a Fleet-format
//! theme (`oak-theme`) and turned into gpui values once.
//!
//! The bundled themes under `assets/themes` carry the values JetBrains
//! Fleet 1.48 computes for its own `dark`, `dark_purple` and `light` themes,
//! read off its theme files on 2026-11-01 (`docs/fleet-probe.md`), keyed by
//! Fleet's own names so a theme file written for Fleet loads here unchanged.
//! Every token below names the Fleet key it comes from.

use crate::assets::{self, MONO_FONT, UI_FONT};
use anyhow::{anyhow, Context as _, Result};
use gpui::{FontWeight, Global, Hsla, Rgba, SharedString};
use oak_theme::Color;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Fleet's sizes, in logical pixels. Measured on Fleet 1.48.261 rendering at
/// 2.76 device pixels per logical pixel (display 1.38 × zoom 2.0), see
/// `docs/fleet-probe.md` for the raw numbers and what is still estimated.
pub mod metrics {
    /// The row above the islands: the logo, the panel toggles, the workspace
    /// name, the run and search controls.
    pub const TOOLBAR_HEIGHT: f32 = 36.;
    pub const TOOLBAR_BUTTON: f32 = 28.;
    pub const TOOLBAR_GAP: f32 = 4.;
    pub const TOOLBAR_PAD_X: f32 = 6.;
    pub const LOGO_SIZE: f32 = 24.;
    pub const ICON_SIZE: f32 = 16.;
    /// The margin between the window edge and an island, and between two islands.
    pub const ISLAND_MARGIN: f32 = 8.;
    pub const ISLAND_RADIUS: f32 = 8.;
    pub const LEFT_PANEL_WIDTH: f32 = 300.;
    pub const BOTTOM_PANEL_HEIGHT: f32 = 220.;
    /// A tool panel's own tab row ("Files", "Search", "Git", "History", "+").
    pub const PANEL_TAB_HEIGHT: f32 = 28.;
    pub const PANEL_TAB_RADIUS: f32 = 6.;
    pub const PANEL_TAB_PAD_X: f32 = 6.;
    pub const PANEL_TAB_GAP: f32 = 2.;
    pub const PANEL_HEADER_PAD_TOP: f32 = 6.;
    pub const PANEL_HEADER_PAD_BOTTOM: f32 = 5.;
    pub const PANEL_PAD_X: f32 = 6.;
    pub const TREE_ROW_HEIGHT: f32 = 26.;
    pub const TREE_ROW_RADIUS: f32 = 4.;
    pub const TREE_ROOT_PAD_LEFT: f32 = 14.;
    pub const TREE_PAD_LEFT: f32 = 5.;
    pub const TREE_INDENT: f32 = 16.;
    pub const TREE_ICON_GAP: f32 = 5.;
    /// The editor island's tab row: tabs 28 tall in a 40 px band.
    pub const EDITOR_TAB_BAR_HEIGHT: f32 = 40.;
    pub const EDITOR_TAB_HEIGHT: f32 = 28.;
    pub const EDITOR_TAB_RADIUS: f32 = 6.;
    pub const EDITOR_TAB_PAD_X: f32 = 8.;
    pub const EDITOR_TAB_GAP: f32 = 4.;
    pub const EDITOR_TAB_ICON_GAP: f32 = 6.;
    /// The gutter: line numbers right-aligned at `GUTTER_NUMBER_RIGHT`, text from `GUTTER_WIDTH`.
    pub const GUTTER_NUMBER_RIGHT: f32 = 36.;
    pub const GUTTER_WIDTH: f32 = 60.;
    pub const EDITOR_FONT_SIZE: f32 = 13.;
    pub const EDITOR_LINE_HEIGHT: f32 = 21.;
    pub const CARET_WIDTH: f32 = 2.;
    pub const SCROLLBAR_WIDTH: f32 = 10.;
    pub const SCROLLBAR_THUMB: f32 = 6.;
    pub const UI_FONT_SIZE: f32 = 13.;
    pub const STATUS_BAR_HEIGHT: f32 = 27.;
    pub const STATUS_PAD_X: f32 = 8.;
    pub const STATUS_GAP: f32 = 24.;
}

pub fn to_hsla(c: Color) -> Hsla {
    let (r, g, b, a) = c.to_f32();
    Rgba { r, g, b, a }.into()
}

/// A syntax attribute ready to paint: colour, weight and slant.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)] // the registry of what a theme offers; surfaces pick from it as they appear
pub struct Syntax {
    pub color: Option<Hsla>,
    pub background: Option<Hsla>,
    pub weight: FontWeight,
    pub italic: bool,
    pub underline: Option<Hsla>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // the registry of what a theme offers; surfaces pick from it as they appear
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub ui_font: SharedString,
    pub mono_font: SharedString,

    // text.*
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    pub text_disabled: Hsla,
    pub text_positive: Hsla,
    pub text_dangerous: Hsla,
    pub text_bright: Hsla,
    // background.*, island.background, border, separator.default
    pub background_primary: Hsla,
    pub background_secondary: Hsla,
    pub island_background: Hsla,
    pub border: Hsla,
    pub border_focused: Hsla,
    pub focus_outline: Hsla,
    pub separator: Hsla,
    // icon.*
    pub icon_primary: Hsla,
    pub icon_secondary: Hsla,
    // ghostButton.off.background.* (the toolbar's icon buttons)
    pub ghost_hovered: Hsla,
    pub ghost_pressed: Hsla,
    // tab.*
    pub tab_bg_hovered: Hsla,
    pub tab_bg_selected: Hsla,
    pub tab_bg_selected_focused: Hsla,
    pub tab_border_selected: Hsla,
    pub tab_border_selected_focused: Hsla,
    pub tab_text: Hsla,
    pub tab_text_selected: Hsla,
    pub tab_close_icon: Hsla,
    pub tab_close_icon_hovered: Hsla,
    // listItem.* and tree.*
    pub list_bg_hovered: Hsla,
    pub list_bg_selected: Hsla,
    pub list_bg_focused: Hsla,
    pub list_text: Hsla,
    pub list_text_secondary: Hsla,
    pub tree_focus_border: Hsla,
    // editor.*
    pub editor_text: Hsla,
    pub editor_caret: Hsla,
    pub current_line: Hsla,
    pub current_line_focused: Hsla,
    pub line_number: Hsla,
    pub line_number_current: Hsla,
    pub whitespace_indicator: Hsla,
    pub fold_icon: Hsla,
    pub selection: Hsla,
    pub selection_focused: Hsla,
    pub search_match_bg: Hsla,
    pub brace_match_bg: Hsla,
    // scrollbar.*
    pub scrollbar_thumb: Hsla,
    pub scrollbar_thumb_hovered: Hsla,
    pub scrollbar_track_hovered: Hsla,
    // statusBar.*, popup.*, tooltip.*
    pub status_text: Hsla,
    pub popup_bg: Hsla,
    pub popup_border: Hsla,
    pub tooltip_bg: Hsla,
    // button.primary.*, toggleButton.on.*
    pub button_primary_bg: Hsla,
    pub button_primary_bg_hovered: Hsla,
    pub button_primary_text: Hsla,
    pub toggle_on_bg: Hsla,
    pub toggle_on_text: Hsla,
    pub smart_mode_enabled: Hsla,
    pub link: Hsla,

    syntax: HashMap<String, Syntax>,
}

impl Global for Theme {}

/// Where a theme comes from: a bundled name or a Fleet-format file.
#[derive(Clone, Debug)]
pub enum ThemeSource {
    Bundled(String),
    File(PathBuf),
}

impl ThemeSource {
    /// `dark`, `dark_purple`, `light`, or a path to a JSON file. A bare name
    /// that is not bundled is looked for in Fleet's own themes folder.
    pub fn parse(spec: &str) -> ThemeSource {
        if assets::THEMES.iter().any(|(n, _)| *n == spec) {
            return ThemeSource::Bundled(spec.to_string());
        }
        let path = Path::new(spec);
        if path.exists() {
            return ThemeSource::File(path.to_path_buf());
        }
        if let Some(dir) = fleet_themes_dir() {
            let candidate = dir.join(format!("{spec}.json"));
            if candidate.exists() {
                return ThemeSource::File(candidate);
            }
        }
        ThemeSource::Bundled(spec.to_string())
    }

    /// What `--theme` and `$OAK_THEME` leave unsaid: the theme Fleet's own
    /// `settings.json` names on this machine, else Fleet's default, `dark`.
    pub fn default_for_machine() -> ThemeSource {
        if let Some(name) = fleet_settings_theme() {
            return ThemeSource::parse(&name);
        }
        ThemeSource::Bundled("dark".into())
    }
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
}

fn fleet_themes_dir() -> Option<PathBuf> {
    Some(config_home()?.join("JetBrains/Fleet/themes"))
}

fn fleet_settings_theme() -> Option<String> {
    let path = config_home()?.join("JetBrains/Fleet/settings.json");
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("theme")?.as_str().map(str::to_string)
}

impl Theme {
    pub fn load(source: &ThemeSource) -> Result<Theme> {
        let json = match source {
            ThemeSource::Bundled(name) => assets::THEMES
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, json)| json.to_string())
                .ok_or_else(|| anyhow!("no bundled theme named {name:?}"))?,
            ThemeSource::File(path) => {
                std::fs::read_to_string(path).with_context(|| path.display().to_string())?
            }
        };
        let parsed =
            oak_theme::Theme::parse(&json).with_context(|| format!("loading theme {source:?}"))?;
        Ok(Theme::from_parsed(parsed))
    }

    pub fn from_parsed(t: oak_theme::Theme) -> Theme {
        let dark = t.is_dark();
        let fallback_text = if dark {
            Color::rgb(0xE0, 0xE1, 0xE4)
        } else {
            Color::rgb(0x18, 0x19, 0x1B)
        };
        let fallback_bg = if dark {
            Color::rgb(0x09, 0x09, 0x09)
        } else {
            Color::rgb(0xFF, 0xFF, 0xFF)
        };
        let c = |key: &str, fallback: Color| to_hsla(t.color_or(key, fallback));
        let first = |keys: &[&str], fallback: Color| {
            to_hsla(keys.iter().find_map(|k| t.color(k)).unwrap_or(fallback))
        };
        let text_primary = t.color_or("text.primary", fallback_text);
        let bg_primary = t.color_or("background.primary", fallback_bg);
        let bg_secondary = t.color_or("background.secondary", bg_primary);
        let transparent = Color::TRANSPARENT;

        let attr = |key: &str| -> Option<Syntax> {
            let a = t.attributes_cascading(key)?;
            Some(Syntax {
                color: a.foreground.map(to_hsla),
                background: a.background.map(to_hsla),
                weight: match a.weight {
                    Some(oak_theme::FontWeight::Bold) => FontWeight::BOLD,
                    Some(oak_theme::FontWeight::SemiBold) => FontWeight::SEMIBOLD,
                    Some(oak_theme::FontWeight::Medium) => FontWeight::MEDIUM,
                    _ => FontWeight::NORMAL,
                },
                italic: a.italic,
                underline: a.decoration.and_then(|d| d.color.map(to_hsla)),
            })
        };
        let syntax = t
            .attribute_keys()
            .filter_map(|k| Some((k.to_string(), attr(k)?)))
            .collect();
        let selection_attr = |key: &str| t.attributes(key).and_then(|a| a.background).map(to_hsla);

        Theme {
            name: t.name.clone(),
            dark,
            ui_font: UI_FONT.into(),
            mono_font: MONO_FONT.into(),
            text_primary: to_hsla(text_primary),
            text_secondary: c("text.secondary", text_primary.with_alpha(0x85)),
            text_tertiary: c("text.tertiary", text_primary.with_alpha(0x59)),
            text_disabled: c("text.disabled", text_primary.with_alpha(0x59)),
            text_positive: c("text.positive", Color::rgb(0x40, 0x9D, 0x78)),
            text_dangerous: c("text.dangerous", Color::rgb(0xEC, 0x5D, 0x6F)),
            text_bright: c("text.bright", Color::rgb(0xFF, 0xFF, 0xFF)),
            background_primary: to_hsla(bg_primary),
            background_secondary: to_hsla(bg_secondary),
            island_background: c("island.background", bg_secondary),
            border: c("border", text_primary.with_alpha(0x21)),
            border_focused: c("border.focused", Color::rgb(0x2A, 0x7D, 0xEB)),
            focus_outline: c("focusOutline", Color::rgb(0x22, 0x42, 0x71)),
            separator: c("separator.default", text_primary.with_alpha(0x26)),
            icon_primary: c("icon.primary", text_primary),
            icon_secondary: c("icon.secondary", Color::rgb(0x7A, 0x7F, 0x86)),
            ghost_hovered: c(
                "ghostButton.off.background.hovered",
                text_primary.with_alpha(0x16),
            ),
            ghost_pressed: c(
                "ghostButton.off.background.pressed",
                text_primary.with_alpha(0x21),
            ),
            tab_bg_hovered: c("tab.background.hovered", text_primary.with_alpha(0x14)),
            tab_bg_selected: c("tab.background.selected", text_primary.with_alpha(0x14)),
            tab_bg_selected_focused: c(
                "tab.background.selectedFocused",
                Color::rgb(0x08, 0x70, 0xE4).with_alpha(0x40),
            ),
            tab_border_selected: c("tab.border.selected", text_primary.with_alpha(0x14)),
            tab_border_selected_focused: c(
                "tab.border.selectedFocused",
                Color::rgb(0x08, 0x70, 0xE4).with_alpha(0x80),
            ),
            tab_text: c("tab.text.default", text_primary),
            tab_text_selected: c("tab.text.selectedFocused", Color::rgb(0xFF, 0xFF, 0xFF)),
            tab_close_icon: c("tab.closeIcon.default", text_primary.with_alpha(0x4D)),
            tab_close_icon_hovered: c("tab.closeIcon.hovered", text_primary.with_alpha(0xB2)),
            list_bg_hovered: c("listItem.background.hovered", text_primary.with_alpha(0x0B)),
            list_bg_selected: c(
                "listItem.background.selected",
                text_primary.with_alpha(0x21),
            ),
            list_bg_focused: c(
                "listItem.background.focused",
                Color::rgb(0x08, 0x70, 0xE4).with_alpha(0x66),
            ),
            list_text: c("listItem.text.default", text_primary),
            list_text_secondary: c("listItem.text.secondary", text_primary.with_alpha(0x85)),
            tree_focus_border: c("tree.focusBorder", Color::rgb(0x2A, 0x7D, 0xEB)),
            editor_text: c("editor.text", text_primary),
            editor_caret: c("editor.caret.background", text_primary),
            current_line: c(
                "editor.currentLine.background.default",
                text_primary.with_alpha(0x1A),
            ),
            current_line_focused: c(
                "editor.currentLine.background.focused",
                Color::rgb(0x08, 0x70, 0xE4).with_alpha(0x33),
            ),
            line_number: c("editor.lineNumber.default", text_primary.with_alpha(0x4D)),
            line_number_current: c("editor.lineNumber.current", text_primary),
            whitespace_indicator: c("editor.whitespaceIndicator", text_primary.with_alpha(0x4D)),
            fold_icon: c(
                "editor.foldIndicator.icon.default",
                Color::rgb(0x7A, 0x7F, 0x86),
            ),
            selection: selection_attr("editor.selection")
                .unwrap_or_else(|| to_hsla(Color::rgb(0x4C, 0x51, 0x57))),
            selection_focused: selection_attr("editor.selection.focused")
                .unwrap_or_else(|| to_hsla(Color::rgb(0x22, 0x50, 0x90))),
            search_match_bg: c("search.match.background", Color::rgb(0x6C, 0x4B, 0x18)),
            brace_match_bg: selection_attr("editor.brace.match")
                .unwrap_or_else(|| to_hsla(Color::rgb(0x2A, 0x52, 0x4F))),
            scrollbar_thumb: c("scrollbar.thumb.default", text_primary.with_alpha(0x26)),
            scrollbar_thumb_hovered: c("scrollbar.thumb.hovered", text_primary.with_alpha(0x33)),
            scrollbar_track_hovered: c("scrollbar.track.hovered", text_primary.with_alpha(0x1A)),
            status_text: c("statusBar.text.default", text_primary.with_alpha(0x80)),
            popup_bg: c("popup.background", bg_secondary),
            popup_border: c("popup.border", text_primary.with_alpha(0x21)),
            tooltip_bg: c("tooltip.background", Color::rgb(0x32, 0x34, 0x38)),
            button_primary_bg: c(
                "button.primary.background.default",
                Color::rgb(0x08, 0x70, 0xE4),
            ),
            button_primary_bg_hovered: c(
                "button.primary.background.hovered",
                Color::rgb(0x18, 0x68, 0xCB),
            ),
            button_primary_text: c("button.primary.text.default", Color::rgb(0xFF, 0xFF, 0xFF)),
            toggle_on_bg: c(
                "toggleButton.on.background.default",
                Color::rgb(0x08, 0x70, 0xE4),
            ),
            toggle_on_text: c("toggleButton.on.text.default", Color::rgb(0xFF, 0xFF, 0xFF)),
            smart_mode_enabled: c("smartMode.background.enabled", Color::rgb(0x2A, 0x7D, 0xEB)),
            link: first(&["link.text.default"], Color::rgb(0x4B, 0x8D, 0xEC)),
            syntax,
        }
        .with_transparent_fallbacks(transparent)
    }

    fn with_transparent_fallbacks(self, _t: Color) -> Theme {
        self
    }

    /// The attribute a highlighter capture paints as, by Fleet key, with the
    /// dotted-name cascade already applied when the theme was loaded.
    pub fn syntax(&self, key: &str) -> Option<&Syntax> {
        if let Some(s) = self.syntax.get(key) {
            return Some(s);
        }
        let mut key = key;
        while let Some((parent, _)) = key.rsplit_once('.') {
            if let Some(s) = self.syntax.get(parent) {
                return Some(s);
            }
            key = parent;
        }
        None
    }
}
