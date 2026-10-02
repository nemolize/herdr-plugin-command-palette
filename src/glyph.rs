//! Each glyph is East Asian Width `N`, like the icons (docs/design.md §4): an `A`
//! one is a cell to ratatui but two in a CJK-locale terminal, shifting its row.

use unicode_width::UnicodeWidthStr;

/// Marks the selected row. Not `▶`, which is `A`.
pub const HIGHLIGHT_SYMBOL: &str = "▸ ";

/// Not `·`, which is `A`.
pub const SEPARATOR: &str = " ⋅ ";

/// Not `▏`, which is `A`.
pub const CURSOR: &str = "⎸";

/// Whether `text` holds no East Asian Ambiguous character: its width is the
/// same whether Ambiguous characters count as one cell or two.
pub fn same_width_in_every_locale(text: &str) -> bool {
    text.width() == text.width_cjk()
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::{same_width_in_every_locale, CURSOR, HIGHLIGHT_SYMBOL, SEPARATOR};

    #[test]
    fn every_glyph_takes_the_same_cells_in_every_locale() {
        for (glyph, cells) in [(HIGHLIGHT_SYMBOL, 2), (SEPARATOR, 3), (CURSOR, 1)] {
            assert_eq!(glyph.width(), cells, "{glyph:?}");
            assert!(same_width_in_every_locale(glyph), "{glyph:?}");
        }
    }

    #[test]
    fn an_ambiguous_character_is_told_apart() {
        for ambiguous in ["…", "·", "▏", "—", "▶"] {
            assert!(!same_width_in_every_locale(ambiguous), "{ambiguous:?}");
        }
        assert!(same_width_in_every_locale("New tab..."));
    }
}
