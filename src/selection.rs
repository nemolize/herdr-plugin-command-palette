//! Filtering, ranking and cursor state for a list of rows — the machinery every
//! stage shares, independent of which stage is up (docs/design.md §7).
use ratatui::widgets::ListState;

use crate::fuzzy;

/// A query over a set of rows, and the cursor into what survived it.
///
/// Rows are supplied per call rather than held: the stage owns them, and they
/// are different types (catalog entries, target rows) between stages.
#[derive(Default)]
pub struct Selection {
    pub query: String,
    pub state: ListState,
    filtered: Vec<usize>,
}

impl Selection {
    /// Re-runs the query over `rows`, ordering ties by `rank`.
    ///
    /// The query filters and `rank` only orders what survives it, so a
    /// frequently-used command never outranks a better textual match (§7).
    pub fn refilter<F>(&mut self, rows: &[&str], rank: F)
    where
        F: Fn(usize) -> f64,
    {
        let mut scored: Vec<(i32, i64, usize)> = rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                fuzzy::score(&self.query, row).map(|s| (s, (rank(i) * 1000.0) as i64, i))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        self.filtered = scored.into_iter().map(|(_, _, i)| i).collect();
        self.state.select((!self.filtered.is_empty()).then_some(0));
    }

    pub fn clear_query(&mut self) {
        self.query.clear();
    }

    pub fn shown(&self) -> usize {
        self.filtered.len()
    }

    /// The row indices that survived the query, in display order.
    pub fn visible(&self) -> &[usize] {
        &self.filtered
    }

    /// The index into the caller's rows, not into the filtered view.
    pub fn selected(&self) -> Option<usize> {
        self.state
            .selected()
            .and_then(|i| self.filtered.get(i))
            .copied()
    }

    pub fn move_by(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as i32;
        let last = self.filtered.len() as i32 - 1;
        self.state
            .select(Some((cur + delta).clamp(0, last) as usize));
    }
}

#[cfg(test)]
mod tests {
    use super::Selection;

    fn sel(rows: &[&str], query: &str) -> Selection {
        let mut s = Selection {
            query: query.to_string(),
            ..Default::default()
        };
        s.refilter(rows, |_| 0.0);
        s
    }

    #[test]
    fn a_query_keeps_only_the_rows_it_matches() {
        let s = sel(&["Split pane: right", "New tab"], "split");
        assert_eq!(s.visible(), &[0]);
        assert_eq!(s.shown(), 1);
    }

    /// Rank orders ties; it never rescues a row the query rejected.
    #[test]
    fn rank_orders_survivors_but_cannot_admit_a_non_match() {
        let rows = ["New tab", "Split pane: right"];
        let mut s = Selection {
            query: "split".into(),
            ..Default::default()
        };
        s.refilter(&rows, |i| if i == 0 { 50.0 } else { 0.0 });
        assert_eq!(s.visible(), &[1], "heavily-ranked non-match stays out");

        s.clear_query();
        s.refilter(&rows, |i| if i == 0 { 50.0 } else { 0.0 });
        assert_eq!(s.visible()[0], 0, "and leads once it matches");
    }

    #[test]
    fn the_cursor_stays_inside_the_filtered_rows() {
        let mut s = sel(&["Alpha", "Beta"], "");
        s.move_by(-5);
        assert_eq!(s.state.selected(), Some(0));
        s.move_by(5);
        assert_eq!(s.state.selected(), Some(1));
    }

    /// `selected` indexes the caller's rows, so a filtered-out row cannot be
    /// confused for the one under the cursor.
    #[test]
    fn selected_maps_back_to_the_original_row() {
        let s = sel(&["New tab", "Split pane: right"], "split");
        assert_eq!(s.selected(), Some(1));
    }

    #[test]
    fn an_empty_result_selects_nothing() {
        let s = sel(&["Alpha"], "zzzz");
        assert!(s.visible().is_empty());
        assert_eq!(s.selected(), None);
    }
}
