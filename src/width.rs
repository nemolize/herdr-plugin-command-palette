//! How wide text is once drawn: the clusters it breaks into, the glyph each is
//! drawn as, and the whole-cluster clips that fit a run of them into a room.

use ratatui::buffer::CellWidth;
use unicode_segmentation::UnicodeSegmentation;

use crate::glyph::{control_picture, STAND_IN};

/// The drawn form of each cluster of `text`, the clusters `LineEdit` puts its
/// cursor stops between, so every stop falls between two drawn glyphs.
pub fn drawn_clusters(text: &str) -> Vec<String> {
    text.graphemes(true).map(drawn_cluster).collect()
}

/// `cluster` with every control character as its Control Picture, so none
/// reaches `CellWidth`, which panics on a raw one in a debug build. A cluster
/// that would still draw nothing goes onto `STAND_IN`.
fn drawn_cluster(cluster: &str) -> String {
    let drawn: String = cluster
        .chars()
        .map(|c| {
            if c.is_control() {
                control_picture(c)
            } else {
                c
            }
        })
        .collect();
    if drawn_width(&drawn) > 0 {
        return drawn;
    }
    // A format character such as U+200B is a cluster break of its own, so it
    // would not ride on the stand-in the way a mark does; it gives way to it.
    let based = format!("{STAND_IN}{drawn}");
    if based.graphemes(true).count() == 1 {
        based
    } else {
        STAND_IN.to_string()
    }
}

pub fn clusters_width(clusters: &[String]) -> u16 {
    clusters.iter().map(|c| drawn_width(c)).sum()
}

/// Cells `text` occupies once drawn, per ratatui's own per-cell measurement.
///
/// `Line::width` is NOT this number — it reports 1 for `ｶ\u{FF9E}`, which the
/// buffer lays out in 2. Summing scalars is wrong too, in both directions.
pub fn drawn_width(text: &str) -> u16 {
    // Dropped rather than measured because `CellWidth` panics on one in a debug
    // build, and a catalog title is user-written.
    text.graphemes(true)
        .filter(|cluster| !cluster.chars().any(char::is_control))
        .map(|cluster| cluster.cell_width())
        .sum()
}

/// The last of `clusters` that fit in `room` columns. Whole clusters only:
/// cutting inside one draws a DIFFERENT glyph, or `␊` without its `␍`.
pub fn tail_within(clusters: &[String], room: u16) -> &[String] {
    let mut used = 0;
    let kept = clusters
        .iter()
        .rev()
        .take_while(|c| {
            used += drawn_width(c);
            used <= room
        })
        .count();
    &clusters[clusters.len() - kept..]
}

pub fn head_within(clusters: &[String], room: u16) -> &[String] {
    let mut used = 0;
    let kept = clusters
        .iter()
        .take_while(|c| {
            used += drawn_width(c);
            used <= room
        })
        .count();
    &clusters[..kept]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyph::{HIGHLIGHT_COLUMNS, HIGHLIGHT_SYMBOL};

    /// Issue #110: every row is laid out against `HIGHLIGHT_COLUMNS`, so a marker
    /// a CJK-locale terminal draws wider shifts the selected row alone.
    #[test]
    fn the_highlight_symbol_is_drawn_in_its_columns() {
        assert_eq!(drawn_width(HIGHLIGHT_SYMBOL), HIGHLIGHT_COLUMNS);
    }

    /// Slices of drawn clusters, `␍␊` being one cluster of two cells.
    fn clusters(parts: &[&str]) -> Vec<String> {
        parts.iter().map(ToString::to_string).collect()
    }

    /// Widths 1, 2, 2, 1: a room is filled exactly, or a cluster is left out
    /// whole, never cut.
    #[test]
    fn the_tail_within_a_room_is_whole_clusters_from_the_end() {
        let all = clusters(&["a", "あ", "␍␊", "b"]);
        let cases: &[(u16, &[&str])] = &[
            (0, &[]),
            (1, &["b"]),
            (2, &["b"]),
            (3, &["␍␊", "b"]),
            (4, &["␍␊", "b"]),
            (5, &["あ", "␍␊", "b"]),
            (6, &["a", "あ", "␍␊", "b"]),
            (7, &["a", "あ", "␍␊", "b"]),
        ];

        for (room, expected) in cases {
            assert_eq!(tail_within(&all, *room), clusters(expected), "room {room}");
        }
    }

    #[test]
    fn the_head_within_a_room_is_whole_clusters_from_the_start() {
        let all = clusters(&["a", "あ", "␍␊", "b"]);
        let cases: &[(u16, &[&str])] = &[
            (0, &[]),
            (1, &["a"]),
            (2, &["a"]),
            (3, &["a", "あ"]),
            (4, &["a", "あ"]),
            (5, &["a", "あ", "␍␊"]),
            (6, &["a", "あ", "␍␊", "b"]),
            (7, &["a", "あ", "␍␊", "b"]),
        ];

        for (room, expected) in cases {
            assert_eq!(head_within(&all, *room), clusters(expected), "room {room}");
        }
    }
}
