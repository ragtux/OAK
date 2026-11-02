//! Keymaps in JetBrains Fleet's JSON format, so Oak's bindings are written
//! in the same file a Fleet user already edits (`user.json`'s `keymap`
//! array) and a Fleet keymap drops in unchanged.
//!
//! The format is `{ "keymap": [ { "key": "ctrl-f4", "mac": "cmd-w",
//! "linux": ..., "win": ..., "action": "close-tab" }, ... ] }`. `key` is
//! the chord on every platform that has no more specific entry; `mac`,
//! `win` and `linux` override it. Chords are dash-joined modifier names
//! (`ctrl`, `alt`, `shift`, `cmd`, `meta`) and one key name, Fleet's
//! spelling (`period`, `leftbracket`, `digit7`, `numpadenter`, `return`).
//! Fleet allows `//` comments and trailing commas, so the parser strips
//! them first.

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Linux,
    Mac,
    Windows,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(target_os = "macos") {
            Platform::Mac
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else {
            Platform::Linux
        }
    }
}

/// One entry of the `keymap` array.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Binding {
    pub key: Option<String>,
    pub mac: Option<String>,
    pub win: Option<String>,
    pub linux: Option<String>,
    pub action: String,
    /// Fleet's optional `when` context; kept verbatim, unused for now.
    #[serde(default)]
    pub when: Option<String>,
}

impl Binding {
    /// The chord this entry means on `platform`, if any.
    pub fn chord_for(&self, platform: Platform) -> Option<&str> {
        let specific = match platform {
            Platform::Linux => &self.linux,
            Platform::Mac => &self.mac,
            Platform::Windows => &self.win,
        };
        specific.as_deref().or(self.key.as_deref())
    }
}

/// A binding resolved for one platform, in both spellings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// Fleet's spelling, for display and for writing back.
    pub chord: String,
    /// gpui's spelling (`ctrl-shift-[`), for `KeyBinding::new`.
    pub keystroke: String,
    pub action: String,
}

#[derive(Deserialize)]
struct File {
    #[serde(default)]
    keymap: Vec<Binding>,
}

#[derive(Clone, Debug, Default)]
pub struct Keymap {
    pub bindings: Vec<Binding>,
}

/// Removes `//` line comments (outside strings) and trailing commas.
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
    // Trailing commas: a comma followed only by whitespace and a closer.
    let mut cleaned = String::with_capacity(out.len());
    let bytes: Vec<char> = out.chars().collect();
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '"' && (i == 0 || bytes[i - 1] != '\\') {
            in_string = !in_string;
        }
        if c == ',' && !in_string {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == '}' || bytes[j] == ']') {
                i += 1;
                continue;
            }
        }
        cleaned.push(c);
        i += 1;
    }
    cleaned
}

impl Keymap {
    pub fn parse(src: &str) -> Result<Keymap> {
        let file: File = serde_json::from_str(&strip_json5(src)).context("keymap JSON")?;
        Ok(Keymap {
            bindings: file.keymap,
        })
    }

    /// Every binding that applies on `platform`, in file order. Chords that
    /// gpui cannot express (mouse buttons) are left out.
    pub fn resolved(&self, platform: Platform) -> Vec<Resolved> {
        self.bindings
            .iter()
            .filter_map(|b| {
                let chord = b.chord_for(platform)?;
                let keystroke = chord_to_gpui_on(chord, platform)?;
                Some(Resolved {
                    chord: chord.to_string(),
                    keystroke,
                    action: b.action.clone(),
                })
            })
            .collect()
    }

    /// Adds or overrides bindings with those of `other` (a user keymap over
    /// the default one).
    pub fn merge(&mut self, other: Keymap) {
        self.bindings.extend(other.bindings);
    }
}

/// Fleet's key names that gpui spells differently.
fn key_name(fleet: &str) -> Option<String> {
    let k = fleet.to_ascii_lowercase();
    let mapped = match k.as_str() {
        "return" | "numpadenter" => "enter",
        "escape" | "esc" => "escape",
        "backspace" => "backspace",
        "delete" | "del" => "delete",
        "tab" => "tab",
        "space" => "space",
        "up" | "down" | "left" | "right" | "home" | "end" | "insert" => k.as_str(),
        "pageup" | "page_up" => "pageup",
        "pagedown" | "page_down" => "pagedown",
        "period" | "numpaddecimal" => ".",
        "comma" => ",",
        "slash" | "numpaddivide" => "/",
        "backslash" => "\\",
        "semicolon" => ";",
        "quote" | "apostrophe" => "'",
        "backquote" | "grave" | "graveaccent" | "backtick" => "`",
        "minus" | "numpadsubtract" => "-",
        "equals" | "equal" => "=",
        "plus" | "numpadadd" => "+",
        "numpadmultiply" | "asterisk" => "*",
        "leftbracket" | "openbracket" => "[",
        "rightbracket" | "closebracket" => "]",
        "mouse4" | "mouse5" | "mouse3" | "mouse1" | "mouse2" => return None,
        _ => {
            if let Some(d) = k.strip_prefix("digit") {
                return Some(d.to_string());
            }
            if let Some(d) = k.strip_prefix("numpad") {
                return d.chars().all(|c| c.is_ascii_digit()).then(|| d.to_string());
            }
            if k.len() == 1 || (k.starts_with('f') && k[1..].chars().all(|c| c.is_ascii_digit())) {
                return Some(k);
            }
            return None;
        }
    };
    Some(mapped.to_string())
}

/// `ctrl-shift-leftbracket` → `ctrl-shift-[`. Returns `None` for chords
/// gpui has no spelling for. A two-chord sequence (`ctrl-shift-g
/// ctrl-shift-a`, Fleet's git keys) becomes gpui's space-separated one.
pub fn chord_to_gpui(chord: &str) -> Option<String> {
    chord_to_gpui_on(chord, Platform::Mac)
}

/// [`chord_to_gpui`] for a platform: on Windows and Linux gpui reports a
/// shifted symbol key as the shifted character without Shift (Ctrl+Shift+`
/// arrives as `ctrl-~`), so `ctrl-shift-graveaccent` becomes `ctrl-~` there.
pub fn chord_to_gpui_on(chord: &str, platform: Platform) -> Option<String> {
    let fold = platform != Platform::Mac;
    let mut chords = chord.split_whitespace();
    let first = chords.next()?;
    let mut out = single_chord_to_gpui(first, fold)?;
    for next in chords {
        out.push(' ');
        out.push_str(&single_chord_to_gpui(next, fold)?);
    }
    Some(out)
}

/// The character Shift puts on a US keyboard's symbol or digit key.
fn shifted_symbol(key: &str) -> Option<&'static str> {
    Some(match key {
        "`" => "~",
        "1" => "!",
        "2" => "@",
        "3" => "#",
        "4" => "$",
        "5" => "%",
        "6" => "^",
        "7" => "&",
        "8" => "*",
        "9" => "(",
        "0" => ")",
        "-" => "_",
        "=" => "+",
        "[" => "{",
        "]" => "}",
        "\\" => "|",
        ";" => ":",
        "'" => "\"",
        "," => "<",
        "." => ">",
        "/" => "?",
        _ => return None,
    })
}

fn single_chord_to_gpui(chord: &str, fold_shift: bool) -> Option<String> {
    let mut parts: Vec<&str> = chord.split('-').collect();
    // A chord ending in the "-" key itself splits into an empty last part.
    if parts.last() == Some(&"") && parts.len() >= 2 {
        parts.pop();
        let n = parts.len();
        parts[n - 1] = "minus";
        if parts[n - 1].is_empty() {
            return None;
        }
    }
    let (key, mods) = parts.split_last()?;
    let key = key_name(key)?;
    let mut out = String::new();
    let mut seen = Vec::new();
    let mut mods: Vec<&str> = mods.to_vec();
    let shifted = fold_shift
        .then(|| shifted_symbol(&key))
        .flatten()
        .filter(|_| mods.iter().any(|m| m.eq_ignore_ascii_case("shift")));
    if shifted.is_some() {
        mods.retain(|m| !m.eq_ignore_ascii_case("shift"));
    }
    for m in mods {
        let m = match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => "ctrl",
            "alt" | "option" => "alt",
            "shift" => "shift",
            "cmd" | "command" | "meta" | "super" | "win" => "cmd",
            _ => return None,
        };
        if !seen.contains(&m) {
            seen.push(m);
            out.push_str(m);
            out.push('-');
        }
    }
    match shifted {
        Some(s) => out.push_str(s),
        None => out.push_str(&key),
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chords() {
        assert_eq!(
            chord_to_gpui_on("ctrl-shift-graveaccent", Platform::Windows).as_deref(),
            Some("ctrl-~")
        );
        assert_eq!(
            chord_to_gpui_on("ctrl-shift-graveaccent", Platform::Mac).as_deref(),
            Some("ctrl-shift-`")
        );
        assert_eq!(
            chord_to_gpui_on("ctrl-shift-a", Platform::Windows).as_deref(),
            Some("ctrl-shift-a")
        );
        assert_eq!(
            chord_to_gpui("ctrl-shift-leftbracket").as_deref(),
            Some("ctrl-shift-[")
        );
        assert_eq!(
            chord_to_gpui("cmd-alt-shift-digit7").as_deref(),
            Some("cmd-alt-shift-7")
        );
        assert_eq!(
            chord_to_gpui("shift-numpadenter").as_deref(),
            Some("shift-enter")
        );
        assert_eq!(chord_to_gpui("ctrl-period").as_deref(), Some("ctrl-."));
        assert_eq!(chord_to_gpui("f4").as_deref(), Some("f4"));
        assert_eq!(chord_to_gpui("ctrl-minus").as_deref(), Some("ctrl--"));
        assert_eq!(chord_to_gpui("mouse4"), None);
        assert_eq!(
            chord_to_gpui("ctrl-shift-g ctrl-shift-a").as_deref(),
            Some("ctrl-shift-g ctrl-shift-a")
        );
        assert_eq!(chord_to_gpui("ctrl-shift-g mouse4"), None);
        assert_eq!(chord_to_gpui(""), None);
    }

    #[test]
    fn parses_fleet_style_json_and_picks_the_platform() {
        let src = r#"{
          // Fleet allows comments
          "keymap": [
            { "mac": "cmd-w", "key": "ctrl-f4", "action": "close-tab" },
            { "win": "ctrl-w", "linux": "ctrl-w", "action": "close-tab" },
            { "mac": "cmd-l", "action": "terminal/clear" },
            { "key": "mouse4", "action": "back", },
          ],
        }"#;
        let km = Keymap::parse(src).unwrap();
        assert_eq!(km.bindings.len(), 4);
        let linux = km.resolved(Platform::Linux);
        assert_eq!(linux.len(), 2);
        assert_eq!(linux[0].keystroke, "ctrl-f4");
        assert_eq!(linux[1].keystroke, "ctrl-w");
        let mac = km.resolved(Platform::Mac);
        assert_eq!(
            mac.iter().map(|r| r.keystroke.as_str()).collect::<Vec<_>>(),
            ["cmd-w", "cmd-l"]
        );
    }
}
