//! Subsequence match; score rewards earlier and tighter matches.

pub fn score(query: &str, target: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let t: Vec<char> = target.to_lowercase().chars().collect();
    let (mut qi, mut score, mut prev) = (0usize, 0i32, -1i32);
    for (ti, ch) in t.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if *ch == q[qi] {
            if prev >= 0 && ti as i32 == prev + 1 {
                score += 5;
            }
            score -= (ti / 10) as i32;
            prev = ti as i32;
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score)
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn empty_query_matches_everything_at_zero() {
        assert_eq!(score("", "Split pane: right"), Some(0));
    }

    #[test]
    fn non_subsequence_does_not_match() {
        assert_eq!(score("zzz", "Split pane: right"), None);
    }

    #[test]
    fn matches_are_case_insensitive() {
        assert!(score("SPLIT", "Split pane: right").is_some());
    }

    #[test]
    fn contiguous_beats_scattered() {
        let contiguous = score("split", "Split pane: right").unwrap();
        let scattered = score("split", "Show pliant interface tools").unwrap();
        assert!(contiguous > scattered, "{contiguous} !> {scattered}");
    }

    #[test]
    fn earlier_match_beats_later_one() {
        let early = score("tab", "Tab: new").unwrap();
        let late = score("tab", "Move pane to the tab").unwrap();
        assert!(early > late, "{early} !> {late}");
    }
}
