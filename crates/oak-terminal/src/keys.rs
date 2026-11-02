//! Keys to the bytes a terminal application reads: xterm's sequences, with
//! the cursor-key mode an application may set. Printable keys without
//! Control or Alt are not encoded here: the platform's text input delivers
//! them (dead keys and IME included), so the two paths never both fire.

/// The modifier keys held with a key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
}

/// The terminal modes that change what a key sends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct KeyModes {
    /// DECCKM: arrows and Home/End send `ESC O x` instead of `ESC [ x`.
    pub app_cursor: bool,
    /// Alt is Meta: Alt+key sends ESC then the key (`terminal.useMacOptionAsMeta`
    /// on macOS; always elsewhere).
    pub alt_is_meta: bool,
}

/// The bytes for a key named as gpui spells it (`"enter"`, `"up"`, `"f5"`, a
/// letter), with `key_char` the text the key would type; `None` leaves the
/// key to the text input or to nobody.
pub fn encode(
    key: &str,
    key_char: Option<&str>,
    mods: Modifiers,
    modes: KeyModes,
) -> Option<Vec<u8>> {
    let alt = mods.alt && modes.alt_is_meta;
    let with_alt = |mut bytes: Vec<u8>| {
        if alt {
            bytes.insert(0, 0x1b);
        }
        Some(bytes)
    };
    // xterm's modifier parameter: 1 + Shift(1) + Alt(2) + Control(4).
    let param = 1 + mods.shift as u8 + (alt as u8) * 2 + (mods.control as u8) * 4;
    let csi = |suffix: &str| -> Option<Vec<u8>> {
        if param == 1 {
            Some(format!("\x1b[{suffix}").into_bytes())
        } else {
            Some(format!("\x1b[1;{param}{suffix}").into_bytes())
        }
    };
    let cursor = |letter: char| -> Option<Vec<u8>> {
        if param == 1 && modes.app_cursor {
            Some(format!("\x1bO{letter}").into_bytes())
        } else {
            csi(&letter.to_string())
        }
    };
    let tilde = |code: u8| -> Option<Vec<u8>> {
        if param == 1 {
            Some(format!("\x1b[{code}~").into_bytes())
        } else {
            Some(format!("\x1b[{code};{param}~").into_bytes())
        }
    };
    let function = |letter: char| -> Option<Vec<u8>> {
        if param == 1 {
            Some(format!("\x1bO{letter}").into_bytes())
        } else {
            Some(format!("\x1b[1;{param}{letter}").into_bytes())
        }
    };

    match key {
        "enter" => with_alt(vec![b'\r']),
        "tab" if mods.shift => Some(b"\x1b[Z".to_vec()),
        "tab" => with_alt(vec![b'\t']),
        "backspace" if mods.control => with_alt(vec![0x08]),
        "backspace" => with_alt(vec![0x7f]),
        "escape" => Some(vec![0x1b]),
        "up" => cursor('A'),
        "down" => cursor('B'),
        "right" => cursor('C'),
        "left" => cursor('D'),
        "home" => cursor('H'),
        "end" => cursor('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => function('P'),
        "f2" => function('Q'),
        "f3" => function('R'),
        "f4" => function('S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "space" if mods.control => with_alt(vec![0]),
        "space" if alt => Some(b"\x1b ".to_vec()),
        _ if mods.control => {
            let c = key.chars().next().filter(|_| key.chars().count() == 1)?;
            let byte = match c.to_ascii_lowercase() {
                'a'..='z' => c.to_ascii_lowercase() as u8 & 0x1f,
                '@' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '/' | '7' => 0x1f,
                '?' | '8' => 0x7f,
                _ => return None,
            };
            with_alt(vec![byte])
        }
        _ if alt => {
            let text = key_char.filter(|t| !t.is_empty()).unwrap_or(key);
            if text.chars().count() == 1 {
                let mut bytes = vec![0x1b];
                bytes.extend_from_slice(text.as_bytes());
                Some(bytes)
            } else {
                None
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: KeyModes = KeyModes {
        app_cursor: false,
        alt_is_meta: true,
    };

    fn plain(key: &str) -> Option<Vec<u8>> {
        encode(key, None, Modifiers::default(), META)
    }

    #[test]
    fn named_keys() {
        assert_eq!(plain("enter"), Some(b"\r".to_vec()));
        assert_eq!(plain("tab"), Some(b"\t".to_vec()));
        assert_eq!(plain("backspace"), Some(vec![0x7f]));
        assert_eq!(plain("escape"), Some(vec![0x1b]));
        assert_eq!(plain("up"), Some(b"\x1b[A".to_vec()));
        assert_eq!(plain("home"), Some(b"\x1b[H".to_vec()));
        assert_eq!(plain("delete"), Some(b"\x1b[3~".to_vec()));
        assert_eq!(plain("pageup"), Some(b"\x1b[5~".to_vec()));
        assert_eq!(plain("f1"), Some(b"\x1bOP".to_vec()));
        assert_eq!(plain("f5"), Some(b"\x1b[15~".to_vec()));
        assert_eq!(plain("f12"), Some(b"\x1b[24~".to_vec()));
    }

    #[test]
    fn printable_keys_go_to_the_text_input() {
        assert_eq!(plain("a"), None);
        assert_eq!(plain("space"), None);
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        assert_eq!(encode("a", Some("A"), shift, META), None);
    }

    #[test]
    fn control_and_alt() {
        let ctrl = Modifiers {
            control: true,
            ..Default::default()
        };
        assert_eq!(encode("c", None, ctrl, META), Some(vec![0x03]));
        assert_eq!(encode("z", None, ctrl, META), Some(vec![0x1a]));
        assert_eq!(encode("[", None, ctrl, META), Some(vec![0x1b]));
        assert_eq!(encode("space", None, ctrl, META), Some(vec![0]));
        assert_eq!(encode("backspace", None, ctrl, META), Some(vec![0x08]));
        let alt = Modifiers {
            alt: true,
            ..Default::default()
        };
        assert_eq!(encode("b", Some("b"), alt, META), Some(b"\x1bb".to_vec()));
        assert_eq!(encode("enter", None, alt, META), Some(b"\x1b\r".to_vec()));
        let option = KeyModes {
            app_cursor: false,
            alt_is_meta: false,
        };
        assert_eq!(encode("b", Some("∫"), alt, option), None);
    }

    #[test]
    fn modifiers_parameter_and_app_cursor() {
        let shift = Modifiers {
            shift: true,
            ..Default::default()
        };
        let ctrl_shift = Modifiers {
            shift: true,
            control: true,
            alt: false,
        };
        assert_eq!(encode("up", None, shift, META), Some(b"\x1b[1;2A".to_vec()));
        assert_eq!(
            encode("left", None, ctrl_shift, META),
            Some(b"\x1b[1;6D".to_vec())
        );
        assert_eq!(
            encode("delete", None, shift, META),
            Some(b"\x1b[3;2~".to_vec())
        );
        assert_eq!(encode("tab", None, shift, META), Some(b"\x1b[Z".to_vec()));
        assert_eq!(encode("f1", None, shift, META), Some(b"\x1b[1;2P".to_vec()));
        let app = KeyModes {
            app_cursor: true,
            alt_is_meta: true,
        };
        let none = Modifiers::default();
        assert_eq!(encode("up", None, none, app), Some(b"\x1bOA".to_vec()));
        assert_eq!(encode("end", None, none, app), Some(b"\x1bOF".to_vec()));
        assert_eq!(encode("up", None, shift, app), Some(b"\x1b[1;2A".to_vec()));
    }
}
