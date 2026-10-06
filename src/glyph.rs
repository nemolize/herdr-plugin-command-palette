//! Each glyph is East Asian Width `N`, like the icons (docs/design.md §4): an `A`
//! one is a cell to ratatui but two in a CJK-locale terminal, shifting its row.

use unicode_width::UnicodeWidthStr;

/// Marks the selected row. Not `▶`, which is `A`.
pub const HIGHLIGHT_SYMBOL: &str = "▸ ";

pub const HIGHLIGHT_COLUMNS: u16 = 2;

/// Not `·`, which is `A`.
pub const SEPARATOR: &str = " ⋅ ";

/// Not `▏`, which is `A`.
pub const CURSOR: &str = "⎸";

pub const CURSOR_COLUMNS: u16 = 1;

/// Whether `text` holds no East Asian Ambiguous character: its width is the
/// same whether Ambiguous characters count as one cell or two.
pub fn same_width_in_every_locale(text: &str) -> bool {
    text.width() == text.width_cjk()
}

/// The Control Pictures glyph standing in for control character `c`.
pub fn control_picture(c: char) -> char {
    match u32::from(c) {
        n @ 0..=0x1F => char::from_u32(0x2400 + n).unwrap_or('\u{2426}'),
        0x7F => '\u{2421}',
        // C1 controls have no picture of their own.
        _ => '\u{2426}',
    }
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::{
        control_picture, same_width_in_every_locale, CURSOR, CURSOR_COLUMNS, HIGHLIGHT_COLUMNS,
        HIGHLIGHT_SYMBOL, SEPARATOR,
    };

    #[test]
    fn every_glyph_takes_the_same_cells_in_every_locale() {
        for (glyph, cells) in [
            (HIGHLIGHT_SYMBOL, HIGHLIGHT_COLUMNS),
            (SEPARATOR, 3),
            (CURSOR, CURSOR_COLUMNS),
        ] {
            assert_eq!(glyph.width(), usize::from(cells), "{glyph:?}");
            assert!(same_width_in_every_locale(glyph), "{glyph:?}");
        }
    }

    #[test]
    fn every_control_picture_takes_one_cell_in_every_locale() {
        for c in (0..=0x9F)
            .filter_map(char::from_u32)
            .filter(|c| c.is_control())
        {
            let picture = control_picture(c).to_string();
            assert_eq!(picture.width(), 1, "{c:?}");
            assert!(same_width_in_every_locale(&picture), "{c:?}");
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
