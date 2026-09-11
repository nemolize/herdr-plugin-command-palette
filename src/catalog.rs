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
    /// The label to show above a free-text stage that fills the `{text}` in
    /// `args`. Absent means the entry needs no typed argument.
    #[serde(default)]
    pub prompt: Option<String>,
    /// The `[keys]` action name this entry runs the same operation as, so the
    /// palette can show the key that reaches it without the palette. Herdr's
    /// action names are a separate vocabulary from the argv, so nothing derives
    /// this — it is stated here, beside the entry a correction would edit.
    #[serde(default)]
    pub binding: Option<String>,
}

impl Command {
    pub fn needs_target(&self) -> bool {
        self.resolve.is_some()
    }

    pub fn needs_text(&self) -> bool {
        self.prompt.is_some()
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

/// Why an entry cannot be offered, or None when it is well-formed.
///
/// A catalog is user-replaceable, so these hold for hand-written entries too.
/// Each is a mismatch that would otherwise surface as a rename applying the
/// literal string `{text}`, or as a picked target being silently discarded —
/// never as an error.
pub fn rejection(c: &Command) -> Option<String> {
    let texts = c.args.iter().filter(|a| *a == "{text}").count();
    let ids = c.args.iter().filter(|a| *a == "{}").count();

    if c.resolve.is_some() && c.prompt.is_some() {
        return Some("`resolve` and `prompt` on one entry".into());
    }
    match (&c.prompt, texts) {
        (Some(_), 0) => return Some("`prompt` without a `{text}` to fill".into()),
        (Some(_), n) if n > 1 => {
            return Some(format!("{n} `{{text}}` placeholders, expected 1"));
        }
        (None, n) if n > 0 => return Some("`{text}` without a `prompt` to fill it".into()),
        _ => {}
    }
    match (&c.resolve, ids) {
        (Some(_), n) if n != 1 => Some(format!(
            "`resolve` with {n} `{{}}` placeholders, expected 1"
        )),
        (None, n) if n > 0 => Some("`{}` without a `resolve` to fill it".into()),
        _ => None,
    }
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
    use super::{is_older, Catalog, Command};

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

    /// `rejection` covers the placeholder pairing; this is the half it cannot
    /// see — that the subject is a context id `main.rs` can read out of the argv.
    #[test]
    fn a_prompt_entry_names_its_subject_from_the_context() {
        for e in shipped().commands.iter().filter(|e| e.prompt.is_some()) {
            assert!(e.resolve.is_none(), "{}: prompt entry has a picker", e.id);
            let subject = e.args.first().expect("entries have args");
            assert_eq!(
                e.args.get(2).map(String::as_str),
                Some(format!("{{{subject}}}").as_str()),
                "{}: `{}` must name its subject with {{{subject}}}",
                e.id,
                e.args.join(" ")
            );
        }
    }

    /// The shipped entries are the baseline the user's own catalog is judged
    /// against, so none of them may trip the gate that drops an entry.
    #[test]
    fn every_shipped_entry_survives_the_rejection_gate() {
        for e in &shipped().commands {
            assert_eq!(super::rejection(e), None, "{}", e.id);
        }
    }

    /// A hand-written catalog reaches `rejection` too, and each of these would
    /// otherwise rename something to the literal `{text}` or drop a picked id.
    #[test]
    fn a_malformed_entry_is_rejected_with_its_reason() {
        let entry = |args: &[&str], resolve: Option<&str>, prompt: Option<&str>| Command {
            id: "x".into(),
            title: "X".into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            contexts: vec![],
            resolve: resolve.map(str::to_owned),
            prompt: prompt.map(str::to_owned),
            binding: None,
        };

        let both = entry(
            &["tab", "rename", "{}", "{text}"],
            Some("tab list"),
            Some("N"),
        );
        assert!(super::rejection(&both).unwrap().contains("one entry"));

        let no_slot = entry(&["tab", "rename", "x", "y"], None, Some("N"));
        assert!(super::rejection(&no_slot)
            .unwrap()
            .contains("without a `{text}`"));

        let no_prompt = entry(&["tab", "rename", "x", "{text}"], None, None);
        assert!(super::rejection(&no_prompt)
            .unwrap()
            .contains("without a `prompt`"));

        let two_slots = entry(&["tab", "rename", "{text}", "{text}"], None, Some("N"));
        assert!(super::rejection(&two_slots).unwrap().contains("expected 1"));

        let orphan_id = entry(&["tab", "focus", "{}"], None, None);
        assert!(super::rejection(&orphan_id)
            .unwrap()
            .contains("without a `resolve`"));

        assert_eq!(
            super::rejection(&entry(&["tab", "focus", "{}"], Some("tab list"), None)),
            None
        );
    }

    /// `--current` resolves against the SERVER's focused pane. While the palette
    /// is up that is the popup, not the pane it was opened from, so an entry
    /// using it acts on the wrong pane — observed as pane entries doing nothing
    /// when picked. Every pane entry names its target with {pane} instead.
    #[test]
    fn no_entry_targets_a_pane_with_current() {
        for e in &shipped().commands {
            assert!(
                !e.args.iter().any(|a| a == "--current"),
                "{}: `{}` uses --current, which resolves to the palette popup",
                e.id,
                e.args.join(" ")
            );
            if e.args.first().map(String::as_str) == Some("pane") {
                assert!(
                    e.args.iter().any(|a| a == "{pane}"),
                    "{}: a pane entry must name its target with {{pane}}",
                    e.id
                );
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
            // 2 is what an entry must supply, not what the CLI demands: pane's
            // `[LABEL]...` is optional only because `--clear` shares the verb.
            (&["pane", "rename"], 2),
            (&["tab", "create"], 0),
            (&["tab", "focus"], 1),
            (&["tab", "close"], 1),
            (&["tab", "rename"], 2),
            (&["workspace", "create"], 0),
            (&["workspace", "focus"], 1),
            (&["workspace", "close"], 1),
            (&["workspace", "rename"], 2),
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
                    // Only value-taking flags consume the next token. Listed
                    // from the same 0.8.2 `--help` output as `required` above,
                    // and for the same reason: a flag missing here reads its
                    // value as a positional and fails a correct entry, while a
                    // valueless flag wrongly listed eats a real positional and
                    // hides a missing one.
                    if matches!(
                        a.as_str(),
                        "--cwd"
                            | "--direction"
                            | "--env"
                            | "--label"
                            | "--pane"
                            | "--ratio"
                            | "--right-click"
                            | "--source-pane"
                            | "--split"
                            | "--tab"
                            | "--tab-label"
                            | "--target-pane"
                            | "--workspace"
                    ) {
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

    /// A `binding` naming an action herdr does not have shows no key at all,
    /// and the miss is invisible — a blank column is also what an entry with no
    /// counterpart looks like. Held against the names in the running herdr's
    /// own template, so a renamed action fails here rather than going quiet.
    ///
    /// Skipped where no herdr is installed: CI has none, and a missing binary
    /// is not evidence of a drifted catalog.
    #[test]
    fn every_binding_names_an_action_the_running_herdr_declares() {
        let Some(defaults) = std::process::Command::new("herdr")
            .arg("--default-config")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| String::from_utf8(out.stdout).ok())
        else {
            return;
        };

        let bindings = crate::keys::resolve(&defaults, None);
        for e in shipped().commands.iter().filter(|e| e.binding.is_some()) {
            assert_ne!(
                bindings.for_action(e.binding.as_deref()),
                crate::keys::Binding::Unknown,
                "{}: `{}` is not a [keys] action in the running herdr",
                e.id,
                e.binding.as_deref().unwrap()
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
