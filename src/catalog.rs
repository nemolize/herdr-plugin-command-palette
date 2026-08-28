//! The command catalog: data the plugin carries because no API enumerates
//! Herdr's built-in operations (docs/design.md §4).
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub min_herdr_version: Option<String>,
    #[serde(default, rename = "command")]
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub contexts: Vec<String>,
    /// Names the list API whose rows become candidates for the `{}` in `args`.
    /// Absent means the entry runs exactly as written.
    #[serde(default)]
    pub resolve: Option<String>,
}

impl Command {
    pub fn needs_target(&self) -> bool {
        self.resolve.is_some()
    }

    pub fn available_in(&self, context: &str) -> bool {
        self.contexts.is_empty() || self.contexts.iter().any(|c| c == context)
    }
}

/// A user's own catalog overrides the shipped one wholesale rather than merging
/// entry by entry — a merge would leave them unable to remove an entry, which is
/// half of what correcting a drifted catalog means (§4).
pub fn locate(plugin_root: &Path, config_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = config_dir {
        let user = dir.join("catalog.toml");
        if user.is_file() {
            return user;
        }
    }
    plugin_root.join("herdr/catalog.toml")
}

pub fn load(path: &Path) -> Result<Catalog, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let catalog: Catalog =
        toml::from_str(&text).map_err(|e| format!("could not parse {}: {e}", path.display()))?;
    Ok(catalog)
}

/// Compares dotted numeric versions, ignoring any trailing suffix. Returns None
/// when either side is not parseable — an unreadable version is not evidence of
/// a mismatch, so the caller stays quiet rather than warning on a guess.
pub fn is_older(actual: &str, required: &str) -> Option<bool> {
    let parse = |v: &str| -> Option<Vec<u32>> {
        let head = v.split(['-', '+']).next()?;
        head.split('.').map(|p| p.parse::<u32>().ok()).collect()
    };
    let (a, r) = (parse(actual)?, parse(required)?);
    let len = a.len().max(r.len());
    for i in 0..len {
        let (x, y) = (
            a.get(i).copied().unwrap_or(0),
            r.get(i).copied().unwrap_or(0),
        );
        if x != y {
            return Some(x < y);
        }
    }
    Some(false)
}

#[cfg(test)]
mod tests {
    use super::is_older;

    #[test]
    fn detects_an_older_running_herdr() {
        assert_eq!(is_older("0.8.1", "0.8.2"), Some(true));
        assert_eq!(is_older("0.7.9", "0.8.0"), Some(true));
    }

    #[test]
    fn accepts_equal_and_newer() {
        assert_eq!(is_older("0.8.2", "0.8.2"), Some(false));
        assert_eq!(is_older("0.9.0", "0.8.2"), Some(false));
        assert_eq!(is_older("1.0", "0.8.2"), Some(false));
    }

    #[test]
    fn pads_missing_components() {
        assert_eq!(is_older("0.8", "0.8.0"), Some(false));
        assert_eq!(is_older("0.8", "0.8.1"), Some(true));
    }

    #[test]
    fn ignores_a_trailing_suffix() {
        assert_eq!(is_older("0.8.2-rc1", "0.8.2"), Some(false));
    }

    #[test]
    fn unparseable_versions_yield_no_verdict() {
        assert_eq!(is_older("nightly", "0.8.2"), None);
        assert_eq!(is_older("0.8.2", "unknown"), None);
    }
}
