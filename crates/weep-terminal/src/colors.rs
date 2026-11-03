//! The xterm 256-colour table past the 16 ANSI entries, which the theme's
//! `terminal.ansiColors.*` keys cover: the 6×6×6 cube, then 24 greys.

/// The RGB of palette index `i` for `16..=255`; `None` for the 16 ANSI
/// colours, which are the theme's.
pub fn indexed(i: u8) -> Option<(u8, u8, u8)> {
    match i {
        0..=15 => None,
        16..=231 => {
            let n = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + 40 * v };
            Some((level(n / 36 % 6), level(n / 6 % 6), level(n % 6)))
        }
        232..=255 => {
            let v = 8 + 10 * (i - 232);
            Some((v, v, v))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_and_greys() {
        assert_eq!(indexed(0), None);
        assert_eq!(indexed(15), None);
        assert_eq!(indexed(16), Some((0, 0, 0)));
        assert_eq!(indexed(21), Some((0, 0, 255)));
        assert_eq!(indexed(196), Some((255, 0, 0)));
        assert_eq!(indexed(231), Some((255, 255, 255)));
        assert_eq!(indexed(232), Some((8, 8, 8)));
        assert_eq!(indexed(255), Some((238, 238, 238)));
    }
}
