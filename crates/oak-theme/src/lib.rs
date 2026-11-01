//! Themes in JetBrains Fleet's own JSON format, so a theme written for
//! Fleet (`~/.config/JetBrains/Fleet/themes/*.json`) loads in Oak unchanged
//! and the bundled themes are written in the same vocabulary.
//!
//! A file has four tables: `meta` (name, kind, version), `palette` (named
//! colours), `colors` (UI keys such as `tab.background.selected`, each a hex
//! colour or a palette name) and `textAttributes` (editor and syntax keys
//! such as `keyword` or `editor.selection`, each a small object of
//! colours, weight, style and decoration). Hex colours are `#RRGGBB` or
//! `#RRGGBBAA`.
//!
//! This crate only resolves and looks up; the app turns [`Color`] into
//! gpui's `Hsla` and decides which keys it paints from.

use anyhow::{anyhow, bail, Context, Result};
use indexmap::IndexMap;
use serde::Deserialize;

/// An sRGB colour with straight alpha, as the file spells it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const TRANSPARENT: Color = Color {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };

    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }

    /// `#RRGGBB` or `#RRGGBBAA` (Fleet's order: alpha last).
    pub fn parse_hex(s: &str) -> Result<Color> {
        let hex = s
            .strip_prefix('#')
            .ok_or_else(|| anyhow!("not a hex colour: {s:?}"))?;
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).context(s.to_string());
        match hex.len() {
            6 => Ok(Color {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: 255,
            }),
            8 => Ok(Color {
                r: byte(0)?,
                g: byte(2)?,
                b: byte(4)?,
                a: byte(6)?,
            }),
            _ => bail!("hex colour must be 6 or 8 digits: {s:?}"),
        }
    }

    pub fn hex(&self) -> String {
        if self.a == 255 {
            format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
        } else {
            format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
        }
    }

    /// Channels as 0..=1 floats: (r, g, b, a).
    pub fn to_f32(&self) -> (f32, f32, f32, f32) {
        (
            self.r as f32 / 255.,
            self.g as f32 / 255.,
            self.b as f32 / 255.,
            self.a as f32 / 255.,
        )
    }

    pub fn with_alpha(self, a: u8) -> Color {
        Color { a, ..self }
    }
}

/// `theme.kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontWeight {
    Normal,
    Medium,
    SemiBold,
    Bold,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecorationType {
    Underline,
    Strikethrough,
    Overline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecorationStyle {
    Solid,
    Dashed,
    Dotted,
    Wavy,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decoration {
    pub color: Option<Color>,
    pub kind: Option<DecorationType>,
    pub style: Option<DecorationStyle>,
    pub thickness: Option<f32>,
}

/// One `textAttributes` entry, colours resolved.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextAttributes {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
    pub weight: Option<FontWeight>,
    pub italic: bool,
    pub decoration: Option<Decoration>,
    pub scrollbar_mark: Option<Color>,
}

#[derive(Deserialize)]
struct RawDecoration {
    color: Option<String>,
    style: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    thickness: Option<f32>,
}

#[derive(Deserialize)]
struct RawAttributes {
    #[serde(rename = "foregroundColor")]
    foreground: Option<String>,
    #[serde(rename = "backgroundColor")]
    background: Option<String>,
    #[serde(rename = "fontWeight")]
    weight: Option<String>,
    #[serde(rename = "fontStyle")]
    style: Option<String>,
    #[serde(rename = "textDecoration")]
    decoration: Option<RawDecoration>,
    #[serde(rename = "scrollbarMarkColor")]
    scrollbar_mark: Option<String>,
}

#[derive(Deserialize)]
struct RawMeta {
    #[serde(rename = "theme.name")]
    name: Option<String>,
    #[serde(rename = "theme.kind")]
    kind: Option<String>,
}

#[derive(Deserialize)]
struct RawTheme {
    meta: Option<RawMeta>,
    #[serde(default)]
    palette: IndexMap<String, String>,
    #[serde(default)]
    colors: IndexMap<String, String>,
    #[serde(default, rename = "textAttributes")]
    text_attributes: IndexMap<String, RawAttributes>,
}

/// A parsed theme with every reference resolved to a [`Color`].
#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    pub kind: Kind,
    colors: IndexMap<String, Color>,
    attributes: IndexMap<String, TextAttributes>,
}

fn resolve(palette: &IndexMap<String, String>, value: &str) -> Result<Color> {
    if value.starts_with('#') {
        return Color::parse_hex(value);
    }
    if value.eq_ignore_ascii_case("transparent") {
        return Ok(Color::TRANSPARENT);
    }
    // Fleet's files spell palette names both ways: "Neutral_140" and "Neutral 140".
    let spaced = value.replace('_', " ");
    let underscored = value.replace(' ', "_");
    for key in [value, spaced.as_str(), underscored.as_str()] {
        if let Some(hex) = palette.get(key) {
            return Color::parse_hex(hex).with_context(|| format!("palette entry {key:?}"));
        }
    }
    match value {
        "White" => Ok(Color::rgb(255, 255, 255)),
        "Black" => Ok(Color::rgb(0, 0, 0)),
        _ => bail!("unknown colour {value:?}: not hex and not in the palette"),
    }
}

impl Theme {
    pub fn parse(json: &str) -> Result<Theme> {
        let raw: RawTheme = serde_json::from_str(json).context("theme JSON")?;
        let meta = raw.meta.as_ref();
        let name = meta
            .and_then(|m| m.name.clone())
            .unwrap_or_else(|| "Untitled".into());
        let kind = match meta.and_then(|m| m.kind.as_deref()) {
            Some(k) if k.eq_ignore_ascii_case("light") => Kind::Light,
            _ => Kind::Dark,
        };
        let mut colors = IndexMap::with_capacity(raw.colors.len());
        for (key, value) in &raw.colors {
            let color = resolve(&raw.palette, value).with_context(|| format!("colors.{key}"))?;
            colors.insert(key.clone(), color);
        }
        let opt = |v: &Option<String>| -> Result<Option<Color>> {
            v.as_deref().map(|v| resolve(&raw.palette, v)).transpose()
        };
        let mut attributes = IndexMap::with_capacity(raw.text_attributes.len());
        for (key, a) in &raw.text_attributes {
            let decoration = match &a.decoration {
                Some(d) => Some(Decoration {
                    color: opt(&d.color).with_context(|| format!("textAttributes.{key}"))?,
                    kind: d
                        .kind
                        .as_deref()
                        .map(|k| match k.to_ascii_uppercase().as_str() {
                            "STRIKETHROUGH" => DecorationType::Strikethrough,
                            "OVERLINE" => DecorationType::Overline,
                            _ => DecorationType::Underline,
                        }),
                    style: d
                        .style
                        .as_deref()
                        .map(|s| match s.to_ascii_uppercase().as_str() {
                            "DASHED" => DecorationStyle::Dashed,
                            "DOTTED" => DecorationStyle::Dotted,
                            "WAVY" | "WAVE" => DecorationStyle::Wavy,
                            _ => DecorationStyle::Solid,
                        }),
                    thickness: d.thickness,
                }),
                None => None,
            };
            let weight = a
                .weight
                .as_deref()
                .map(|w| match w.to_ascii_uppercase().as_str() {
                    "BOLD" | "EXTRA_BOLD" | "BLACK" => FontWeight::Bold,
                    "SEMI_BOLD" | "SEMIBOLD" => FontWeight::SemiBold,
                    "MEDIUM" => FontWeight::Medium,
                    _ => FontWeight::Normal,
                });
            attributes.insert(
                key.clone(),
                TextAttributes {
                    foreground: opt(&a.foreground)
                        .with_context(|| format!("textAttributes.{key}"))?,
                    background: opt(&a.background)
                        .with_context(|| format!("textAttributes.{key}"))?,
                    weight,
                    italic: a
                        .style
                        .as_deref()
                        .is_some_and(|s| s.eq_ignore_ascii_case("italic")),
                    decoration,
                    scrollbar_mark: opt(&a.scrollbar_mark)?,
                },
            );
        }
        Ok(Theme {
            name,
            kind,
            colors,
            attributes,
        })
    }

    pub fn is_dark(&self) -> bool {
        self.kind == Kind::Dark
    }

    /// A UI colour by its exact key.
    pub fn color(&self, key: &str) -> Option<Color> {
        self.colors.get(key).copied()
    }

    pub fn color_or(&self, key: &str, fallback: Color) -> Color {
        self.color(key).unwrap_or(fallback)
    }

    /// A text attribute by its exact key.
    pub fn attributes(&self, key: &str) -> Option<&TextAttributes> {
        self.attributes.get(key)
    }

    /// A text attribute by the most specific key the theme defines:
    /// `identifier.function.call` falls back to `identifier.function`, then
    /// `identifier`, the way Fleet's own highlighters cascade.
    pub fn attributes_cascading(&self, key: &str) -> Option<&TextAttributes> {
        let mut key = key;
        loop {
            if let Some(a) = self.attributes.get(key) {
                return Some(a);
            }
            key = key.rsplit_once('.')?.0;
        }
    }

    pub fn colors(&self) -> impl Iterator<Item = (&str, Color)> {
        self.colors.iter().map(|(k, c)| (k.as_str(), *c))
    }

    pub fn attribute_keys(&self) -> impl Iterator<Item = &str> {
        self.attributes.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r##"{
      "meta": { "theme.name": "Test", "theme.kind": "Dark", "theme.version": 1 },
      "palette": { "Gray 10": "#111213", "Accent_100": "#2A7DEB", "Half": "#FFFFFF80" },
      "colors": {
        "text.primary": "Gray_10",
        "border": "#3E4147",
        "tab.background.default": "Transparent",
        "listItem.text.secondary": "Half"
      },
      "textAttributes": {
        "keyword": { "foregroundColor": "Accent 100", "fontWeight": "SEMI_BOLD" },
        "identifier": { "foregroundColor": "#E0E1E4" },
        "problem.error": { "textDecoration": { "color": "#EC5D6F", "style": "SOLID", "type": "UNDERLINE", "thickness": 2.0 } }
      }
    }"##;

    #[test]
    fn resolves_palette_names_both_ways() {
        let t = Theme::parse(SAMPLE).unwrap();
        assert_eq!(t.name, "Test");
        assert!(t.is_dark());
        assert_eq!(t.color("text.primary"), Some(Color::rgb(0x11, 0x12, 0x13)));
        assert_eq!(t.color("border"), Some(Color::rgb(0x3E, 0x41, 0x47)));
        assert_eq!(t.color("tab.background.default"), Some(Color::TRANSPARENT));
        assert_eq!(t.color("listItem.text.secondary").unwrap().a, 0x80);
        let kw = t.attributes("keyword").unwrap();
        assert_eq!(kw.foreground, Some(Color::rgb(0x2A, 0x7D, 0xEB)));
        assert_eq!(kw.weight, Some(FontWeight::SemiBold));
    }

    #[test]
    fn attributes_cascade() {
        let t = Theme::parse(SAMPLE).unwrap();
        let a = t.attributes_cascading("identifier.function.call").unwrap();
        assert_eq!(a.foreground, Some(Color::rgb(0xE0, 0xE1, 0xE4)));
        assert!(t.attributes_cascading("nothing.here").is_none());
        let err = t.attributes("problem.error").unwrap().decoration.unwrap();
        assert_eq!(err.kind, Some(DecorationType::Underline));
        assert_eq!(err.thickness, Some(2.0));
    }

    #[test]
    fn unknown_colour_is_an_error() {
        let bad = r#"{ "colors": { "x": "Nope_1" } }"#;
        assert!(Theme::parse(bad).is_err());
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(Color::parse_hex("#0A0B0C").unwrap().hex(), "#0A0B0C");
        assert_eq!(Color::parse_hex("#0a0b0c80").unwrap().hex(), "#0A0B0C80");
        assert!(Color::parse_hex("0A0B0C").is_err());
    }
}
