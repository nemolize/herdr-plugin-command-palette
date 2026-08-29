//! The command catalog: data the plugin carries because no API enumerates
//! Herdr's built-in operations (docs/design.md §4).
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Catalog {
    /// The herdr release the entries were checked against — a soft baseline the
    /// palette warns about, not the manifest's hard install gate.
    #[serde(default)]
    pub checked_against: Option<String>,
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
    use super::{is_older, Catalog};

    fn shipped() -> Catalog {
        // The file the plugin actually ships, not a fixture — a fixture would
        // have passed while `tab.rename` was broken in the real one.
        let text = include_str!("../herdr/catalog.toml");
        toml::from_str(text).expect("shipped catalog parses")
    }

    #[test]
    fn the_shipped_catalog_is_well_formed() {
        let c = shipped();
        assert!(!c.commands.is_empty());
        let mut ids: Vec<&str> = c.commands.iter().map(|e| e.id.as_str()).collect();
        ids.sort_unstable();
        let count = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate command ids");

        for e in &c.commands {
            assert!(!e.title.is_empty(), "{}: no title", e.id);
            assert!(!e.args.is_empty(), "{}: no args", e.id);
            for ctx in &e.contexts {
                assert!(
                    matches!(ctx.as_str(), "global" | "workspace" | "tab" | "pane"),
                    "{}: unknown context {ctx}",
                    e.id
                );
            }
        }
    }

    #[test]
    fn a_resolve_entry_has_exactly_one_placeholder_and_a_supported_list() {
        for e in &shipped().commands {
            let placeholders = e.args.iter().filter(|a| *a == "{}").count();
            match &e.resolve {
                Some(r) => {
                    assert!(
                        matches!(r.as_str(), "pane list" | "tab list" | "workspace list"),
                        "{}: unsupported resolve {r}",
                        e.id
                    );
                    assert_eq!(placeholders, 1, "{}: resolve needs exactly one {{}}", e.id);
                }
                None => assert_eq!(placeholders, 0, "{}: {{}} without a resolve", e.id),
            }
        }
    }

    /// The check that would have caught `tab.rename`, which supplied an id to a
    /// command whose signature is `<TAB_ID> <LABEL>...` and so could never run.
    /// Verifying flags and enum values — as the original catalog check did —
    /// does not reach this: the entry's flags were all valid.
    #[test]
    fn every_entry_supplies_every_positional_its_command_requires() {
        // (argv prefix, how many positionals the CLI requires), from
        // `herdr <sub> <cmd> --help` on 0.8.2.
        let required: &[(&[&str], usize)] = &[
            (&["pane", "split"], 0),
            (&["pane", "focus"], 0),
            (&["pane", "zoom"], 0),
            (&["pane", "swap"], 0),
            (&["pane", "close"], 1),
            (&["pane", "move"], 1),
            (&["tab", "create"], 0),
            (&["tab", "focus"], 1),
            (&["tab", "close"], 1),
            (&["tab", "rename"], 2),
            (&["workspace", "create"], 0),
            (&["workspace", "focus"], 1),
            (&["workspace", "close"], 1),
            (&["server", "reload-config"], 0),
        ];

        for e in &shipped().commands {
            let Some((prefix, needed)) = required
                .iter()
                .filter(|(p, _)| e.args.len() >= p.len() && e.args[..p.len()] == **p)
                .max_by_key(|(p, _)| p.len())
            else {
                panic!("{}: no known signature for `{}`", e.id, e.args.join(" "));
            };

            // Positionals the entry actually supplies: everything after the
            // subcommand that is neither a flag nor a flag's value.
            let mut supplied = 0;
            let mut rest = e.args[prefix.len()..].iter();
            while let Some(a) = rest.next() {
                if a.starts_with("--") {
                    // Only value-taking flags consume the next token; the ones
                    // this catalog uses that do are listed here.
                    if matches!(a.as_str(), "--direction" | "--tab" | "--label") {
                        rest.next();
                    }
                } else {
                    supplied += 1;
                }
            }

            assert_eq!(
                supplied,
                *needed,
                "{}: `{}` needs {needed} positional(s), supplies {supplied}",
                e.id,
                e.args.join(" ")
            );
        }
    }

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
