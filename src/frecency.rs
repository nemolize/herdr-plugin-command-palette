//! Frequency combined with recency, so a command used often and recently sorts
//! above one used often but long ago (docs/design.md §7).
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct Frecency {
    #[serde(default)]
    entries: HashMap<String, Entry>,
}

#[derive(Serialize, Deserialize)]
struct Entry {
    hits: u32,
    last_used: u64,
}

/// Half-life for the recency term. A fortnight puts "used daily last week" above
/// "used heavily last year" without making yesterday's one-off outrank a habit.
const HALF_LIFE_SECS: f64 = 14.0 * 24.0 * 60.0 * 60.0;

pub fn path(state_dir: &Path) -> PathBuf {
    state_dir.join("frecency.json")
}

impl Frecency {
    /// A missing or unreadable file is an empty ranking, never an error: the
    /// palette still works unranked, and refusing to open over corrupt state
    /// would be a worse failure than losing the ordering.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    pub fn record(&mut self, id: &str) {
        let entry = self.entries.entry(id.to_string()).or_insert(Entry {
            hits: 0,
            last_used: 0,
        });
        entry.hits += 1;
        entry.last_used = now();
    }

    pub fn rank(&self, id: &str) -> f64 {
        let Some(entry) = self.entries.get(id) else {
            return 0.0;
        };
        let age = now().saturating_sub(entry.last_used) as f64;
        f64::from(entry.hits) * 0.5_f64.powf(age / HALF_LIFE_SECS)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{Frecency, HALF_LIFE_SECS};

    #[test]
    fn unseen_command_ranks_zero() {
        assert_eq!(Frecency::default().rank("pane.split.right"), 0.0);
    }

    #[test]
    fn more_hits_rank_higher() {
        let mut f = Frecency::default();
        f.record("a");
        f.record("b");
        f.record("b");
        assert!(f.rank("b") > f.rank("a"));
    }

    #[test]
    fn recency_decays_toward_half_at_the_half_life() {
        let mut f = Frecency::default();
        f.record("old");
        // Age the entry by exactly one half-life.
        let entry = f.entries.get_mut("old").unwrap();
        entry.last_used -= HALF_LIFE_SECS as u64;
        let rank = f.rank("old");
        assert!((rank - 0.5).abs() < 0.01, "expected ~0.5, got {rank}");
    }

    #[test]
    fn recent_single_use_outranks_a_stale_heavier_one() {
        let mut f = Frecency::default();
        for _ in 0..4 {
            f.record("stale");
        }
        let entry = f.entries.get_mut("stale").unwrap();
        entry.last_used -= (HALF_LIFE_SECS * 4.0) as u64;
        f.record("fresh");
        assert!(f.rank("fresh") > f.rank("stale"));
    }

    #[test]
    fn round_trips_through_json() {
        let mut f = Frecency::default();
        f.record("pane.split.right");
        let text = serde_json::to_string(&f).unwrap();
        let back: Frecency = serde_json::from_str(&text).unwrap();
        assert!(back.rank("pane.split.right") > 0.0);
    }
}
