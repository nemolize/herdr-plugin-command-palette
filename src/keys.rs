//! The key each entry is reachable by without the palette (docs/design.md §10).
//!
//! Two vocabularies, neither keyed by the catalog's argv. Built-ins are named
//! actions under `[keys]`; plugin actions are `[[keys.command]]` blocks whose
//! `command` is already the palette's own candidate id.
use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

/// What a lookup found, kept apart from "" because the two want different
/// words: an action herdr ships unbound has no shortcut to teach, while one
/// the user cleared is a binding they removed.
#[derive(Debug, PartialEq, Eq)]
pub enum Binding<'a> {
    /// The key to show.
    Key(&'a str),
    /// The action exists and is bound to nothing.
    Unbound,
    /// Nothing claims to know — no `binding` on the entry, or no such action.
    Unknown,
}

#[derive(Debug, Default)]
pub struct Bindings {
    /// Action name -> key, already resolved against the user's overrides.
    actions: HashMap<String, String>,
    /// `<plugin>.<action>` -> key, from the user's `[[keys.command]]` blocks.
    plugin_actions: HashMap<String, String>,
}

impl Bindings {
    /// The key for a built-in, by the action name a catalog entry declares.
    pub fn for_action(&self, action: Option<&str>) -> Binding<'_> {
        let Some(action) = action else {
            return Binding::Unknown;
        };
        match self.actions.get(action) {
            None => Binding::Unknown,
            Some(key) if key.is_empty() => Binding::Unbound,
            Some(key) => Binding::Key(key),
        }
    }

    /// The key for another plugin's action, by the id the palette already
    /// carries. No table stands between the two, so this cannot drift.
    pub fn for_plugin_action(&self, id: &str) -> Binding<'_> {
        match self.plugin_actions.get(id) {
            Some(key) if !key.is_empty() => Binding::Key(key),
            _ => Binding::Unknown,
        }
    }
}

/// The `[keys]` half of a herdr config, as much of it as a key display needs.
#[derive(Debug, Default, Deserialize)]
struct Config {
    #[serde(default)]
    keys: Keys,
}

#[derive(Debug, Default, Deserialize)]
struct Keys {
    /// Every `action = "key"` pair, whichever ones this config sets. Collected
    /// as a flat map rather than named fields so herdr adding an action does
    /// not need a change here — the names are herdr's, not ours.
    #[serde(flatten)]
    actions: HashMap<String, toml::Value>,
    #[serde(default)]
    command: Vec<CommandKey>,
}

#[derive(Debug, Deserialize)]
struct CommandKey {
    #[serde(default)]
    key: String,
    #[serde(default, rename = "type")]
    kind: String,
    #[serde(default)]
    command: String,
}

/// Where herdr itself reads its config from, mirrored rather than guessed:
/// `HERDR_CONFIG_PATH`, else `$XDG_CONFIG_HOME/herdr/config.toml`, else
/// `~/.config/herdr/config.toml`.
///
/// Mirroring a path is a drift surface, so a miss is silent by design — the
/// keys column simply stays empty rather than the palette reporting a file the
/// user never asked it to read.
pub fn config_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HERDR_CONFIG_PATH") {
        return Some(PathBuf::from(path));
    }
    let dir = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(xdg) => PathBuf::from(xdg),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".config"),
    };
    Some(dir.join("herdr").join("config.toml"))
}

/// Resolves the defaults herdr ships against the user's own overrides.
///
/// `defaults` is `herdr --default-config`, whose `[keys]` block lists every
/// action commented out at its default value. Reading them from the running
/// binary is what keeps a default out of this repository, where it would be a
/// second thing to re-check on every herdr release (§4).
pub fn resolve(defaults: &str, user: Option<&str>) -> Bindings {
    let mut actions = default_actions(defaults);
    let mut plugin_actions = HashMap::new();

    if let Some(text) = user {
        let Ok(config) = toml::from_str::<Config>(text) else {
            // Herdr already reported this config at startup and fell back to
            // the same defaults, so re-reporting it here would only duplicate.
            return Bindings {
                actions,
                plugin_actions,
            };
        };
        for (action, value) in config.keys.actions {
            // A non-string is a subtable (`[keys.indexed]`), which would reach
            // the display as a TOML fragment printed beside a title.
            if let Some(key) = value.as_str() {
                actions.insert(action, key.to_string());
            }
        }
        for block in config.keys.command {
            if block.kind == "plugin_action" && !block.command.is_empty() {
                plugin_actions.insert(block.command, block.key);
            }
        }
    }

    Bindings {
        actions,
        plugin_actions,
    }
}

/// The `action = "key"` pairs from the `[keys]` block of a default config.
///
/// Parsed line-wise rather than as TOML because every pair is commented out —
/// the file is a template to copy, so `toml::from_str` sees an empty table.
fn default_actions(defaults: &str) -> HashMap<String, String> {
    let mut actions = HashMap::new();
    let mut in_keys = false;

    for line in defaults.lines() {
        let trimmed = line.trim();
        // `[[keys.command]]` is an example binding rather than a default, so
        // any header ends the block — not only one leaving `[keys]` entirely.
        if trimmed.starts_with('[') {
            in_keys = trimmed == "[keys]";
            continue;
        }
        if !in_keys {
            continue;
        }
        let Some(body) = trimmed.strip_prefix("# ") else {
            continue;
        };
        let Some((name, rest)) = body.split_once(" = ") else {
            continue;
        };
        // Prose wrapped in the comment block reaches here too, and a sentence
        // containing " = " would otherwise land as an action.
        if !name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c == '_' || c.is_ascii_digit())
        {
            continue;
        }
        let Some(value) = quoted(rest) else {
            continue;
        };
        actions.insert(name.to_string(), value.to_string());
    }
    actions
}

/// The contents of the leading `"…"` in `rest`, ignoring the trailing comment
/// that most default lines carry.
fn quoted(rest: &str) -> Option<&str> {
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

#[cfg(test)]
mod tests {
    use super::{resolve, Binding};

    /// The shape of the `[keys]` block in `herdr --default-config`: every pair
    /// commented out, prose between them, and other sections either side.
    const DEFAULTS: &str = r#"
[general]
# version_check = true

[keys]
# Prefix key to enter prefix mode (default: "ctrl+b")
# Accepted key syntax: plain keys, ctrl/shift/alt modifiers.
# prefix = "ctrl+b"
# new_tab = "prefix+c"
# rename_tab = "prefix+shift+t"
# close_tab = "prefix+shift+x"
# open_worktree = ""    # optional, unset by default
# zoom = "prefix+z"       # legacy alias: fullscreen

# [[keys.command]]
# key = "prefix+alt+g"
# command = "lazygit"

[server]
# headless_cols = 120
"#;

    #[test]
    fn a_default_binding_is_read_from_the_running_herdr() {
        let b = resolve(DEFAULTS, None);
        assert_eq!(
            b.for_action(Some("rename_tab")),
            Binding::Key("prefix+shift+t")
        );
        assert_eq!(b.for_action(Some("new_tab")), Binding::Key("prefix+c"));
    }

    /// A trailing comment is prose about the binding, not part of the key.
    #[test]
    fn a_trailing_comment_is_not_part_of_the_key() {
        let b = resolve(DEFAULTS, None);
        assert_eq!(b.for_action(Some("zoom")), Binding::Key("prefix+z"));
    }

    /// Herdr ships some actions unbound, and "no shortcut exists" is not the
    /// same claim as "this action is unknown here".
    #[test]
    fn an_action_herdr_ships_unbound_says_so() {
        let b = resolve(DEFAULTS, None);
        assert_eq!(b.for_action(Some("open_worktree")), Binding::Unbound);
    }

    #[test]
    fn an_action_no_herdr_declares_is_unknown() {
        let b = resolve(DEFAULTS, None);
        assert_eq!(b.for_action(Some("teleport_tab")), Binding::Unknown);
    }

    /// An entry that names no action cannot be looked up at all — which is the
    /// catalog's way of saying "this one has no shortcut to teach".
    #[test]
    fn an_entry_naming_no_action_is_unknown() {
        let b = resolve(DEFAULTS, None);
        assert_eq!(b.for_action(None), Binding::Unknown);
    }

    /// The whole point of reading the user's file: a rebound action must show
    /// the key that actually works, not the one herdr ships.
    #[test]
    fn a_user_override_wins_over_the_default() {
        let user = r#"
[keys]
rename_tab = "prefix+shift+n"
"#;
        let b = resolve(DEFAULTS, Some(user));
        assert_eq!(
            b.for_action(Some("rename_tab")),
            Binding::Key("prefix+shift+n")
        );
        assert_eq!(b.for_action(Some("new_tab")), Binding::Key("prefix+c"));
    }

    /// Clearing a binding is how herdr's own config disables one, so the
    /// cleared action reads as unbound rather than keeping the shipped key.
    #[test]
    fn a_user_clearing_a_binding_unbinds_it() {
        let user = r#"
[keys]
new_tab = ""
"#;
        let b = resolve(DEFAULTS, Some(user));
        assert_eq!(b.for_action(Some("new_tab")), Binding::Unbound);
    }

    /// The half with no table between it and the palette: `command` is already
    /// the candidate id `Candidate::from_action` builds.
    #[test]
    fn a_plugin_action_is_matched_by_the_id_the_palette_already_has() {
        let user = r#"
[[keys.command]]
key = "prefix+r"
type = "plugin_action"
command = "reviewr.toggle"
"#;
        let b = resolve(DEFAULTS, Some(user));
        assert_eq!(
            b.for_plugin_action("reviewr.toggle"),
            Binding::Key("prefix+r")
        );
        assert_eq!(b.for_plugin_action("notes.capture"), Binding::Unknown);
    }

    /// `[[keys.command]]` also carries shell/pane/popup blocks, whose `command`
    /// is a shell line. Without the type check, a block running `lazygit` would
    /// claim the key of any plugin action that happened to share its name.
    #[test]
    fn a_shell_command_block_claims_no_plugin_action() {
        let user = r#"
[[keys.command]]
key = "prefix+alt+g"
type = "popup"
command = "lazygit"
"#;
        let b = resolve(DEFAULTS, Some(user));
        assert_eq!(b.for_plugin_action("lazygit"), Binding::Unknown);
    }

    /// Herdr reports a broken config itself and falls back to defaults; the
    /// palette showing those same defaults is closer to the truth than an
    /// empty column, and is not the place that failure is diagnosed.
    #[test]
    fn an_unparseable_user_config_leaves_the_defaults_standing() {
        let b = resolve(DEFAULTS, Some("this is not [ valid toml"));
        assert_eq!(b.for_action(Some("new_tab")), Binding::Key("prefix+c"));
    }

    /// `[keys.indexed]` is a subtable, not a binding. Flattened into the same
    /// map, it would otherwise print as a TOML fragment beside a title.
    #[test]
    fn a_subtable_under_keys_is_not_a_binding() {
        let user = r#"
[keys.indexed]
tabs = "ctrl"
"#;
        let b = resolve(DEFAULTS, Some(user));
        assert_eq!(b.for_action(Some("indexed")), Binding::Unknown);
    }

    /// A herdr too old to list its keys leaves every built-in unknown rather
    /// than the palette inventing one.
    #[test]
    fn no_defaults_at_all_leaves_every_action_unknown() {
        let b = resolve("", None);
        assert_eq!(b.for_action(Some("new_tab")), Binding::Unknown);
    }
}
