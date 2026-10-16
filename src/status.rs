//! The status message that replaces the footer: how many rows it may take, and
//! the marker drawn when it is cut (docs/design.md §5). The marker is also what
//! the Auto-generate row gets when its text is cut.

use ratatui::buffer::CellWidth;
use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Wrap};

/// The most rows a status message takes, however much room there is.
pub const MAX_STATUS_ROWS: u16 = 5;

/// ASCII rather than `…`, which is East Asian Ambiguous (see `glyph`).
pub const CUT_MARKER: &str = "...";

pub fn paragraph(message: &str) -> Paragraph<'static> {
    Paragraph::new(message.trim_end().to_string())
        .wrap(Wrap { trim: false })
        .dim()
}

/// The rows `status` takes in `rows` rows of `width` columns once `reserved`
/// rows are kept for the rest of the screen, and whether that cuts it.
pub fn fit(status: &Paragraph, width: u16, rows: u16, reserved: u16) -> (u16, bool) {
    let room = rows.saturating_sub(reserved).clamp(1, MAX_STATUS_ROWS);
    let needed = wrapped_height(status, width);
    (needed.min(room), needed > room)
}

/// Counted rather than rendered into a probe: a probe only as tall as the pane
/// misses text after a longer run of blank lines (#151).
fn wrapped_height(status: &Paragraph, width: u16) -> u16 {
    u16::try_from(status.line_count(width))
        .unwrap_or(u16::MAX)
        .max(1)
}

pub fn mark_cut(buffer: &mut Buffer, area: Rect) {
    if area.is_empty() {
        return;
    }
    let y = area.bottom() - 1;
    let marker_width = CUT_MARKER.len() as u16;
    let text_end = (area.left()..area.right())
        .filter(|&x| buffer[(x, y)].symbol().trim() != "")
        .map(|x| x + buffer[(x, y)].symbol().cell_width().max(1))
        .max()
        .unwrap_or(area.left());
    let x = text_end.min(area.right().saturating_sub(marker_width).max(area.left()));
    // The cell left of the marker may hold a wide glyph whose second half the
    // marker now covers; a half-drawn glyph would overprint the marker.
    if x > area.left() && buffer[(x - 1, y)].symbol().cell_width() > 1 {
        buffer[(x - 1, y)].reset();
    }
    let style = buffer[(x.min(area.right() - 1), y)].style();
    buffer.set_stringn(x, y, CUT_MARKER, (area.right() - x) as usize, style);
}
