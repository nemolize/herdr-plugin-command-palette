//! Each glyph is East Asian Width `N`, like the icons (docs/design.md §4): an `A`
//! one is a cell to ratatui but two in a CJK-locale terminal, shifting its row.

/// `·` is `A`.
pub const SEPARATOR: &str = " ⋅ ";

/// `▏` is `A`.
pub const CURSOR: &str = "⎸";

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::{CURSOR, SEPARATOR};

    #[test]
    fn every_glyph_takes_the_same_cells_in_every_locale() {
        for (glyph, cells) in [(SEPARATOR, 3), (CURSOR, 1)] {
            assert_eq!(glyph.width(), cells, "{glyph:?}");
            assert_eq!(glyph.width_cjk(), cells, "{glyph:?}");
        }
    }
}
