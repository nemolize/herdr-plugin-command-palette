//! Switches kept apart from `catalog.toml`, because a user catalog replaces the
//! shipped one wholesale and a switch there would cost a copy of all of it.
use std::io::ErrorKind;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Deserialize;

pub const FILE_NAME: &str = "settings.toml";

#[derive(Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub icons: bool,
    pub auto_name: AutoName,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            icons: true,
            auto_name: AutoName::default(),
        }
    }
}

/// The local LLM behind `Rename workspace...`'s `Auto generate` row.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AutoName {
    pub model: String,
    pub base_url: String,
    pub timeout_secs: u64,
}

impl Default for AutoName {
    fn default() -> Self {
        Self {
            model: "qwen3.5:9b".into(),
            base_url: "http://localhost:11434".into(),
            // Chosen empirically to cover a cold model load plus generation;
            // there is no fixed source for it.
            timeout_secs: TIMEOUT_SECS,
        }
    }
}

const TIMEOUT_SECS: u64 = 60;

impl AutoName {
    /// Why a parsed value cannot be used. Only `http://` is accepted: the client
    /// has no TLS (`Cargo.toml`).
    fn problem(&self) -> Option<String> {
        if self.model.trim().is_empty() {
            Some("auto_name.model is empty".into())
        } else if !self.base_url.starts_with("http://") {
            Some("auto_name.base_url must start with http://".into())
        } else if self.timeout_secs == 0 {
            Some("auto_name.timeout_secs must be above 0".into())
        } else if Instant::now()
            .checked_add(Duration::from_secs(self.timeout_secs))
            .is_none()
        {
            // The HTTP client adds the timeout to the clock and panics on overflow.
            Some("auto_name.timeout_secs is too large".into())
        } else {
            None
        }
    }
}

/// A missing file says nothing; one that exists but cannot be used keeps the
/// defaults and says why, since a misspelt switch ignored silently looks broken.
pub fn load(config_dir: Option<&Path>) -> (Settings, Option<String>) {
    let Some(dir) = config_dir else {
        return (Settings::default(), None);
    };
    let path = dir.join(FILE_NAME);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return (Settings::default(), None),
        Err(e) => return (Settings::default(), Some(unusable(&e.to_string()))),
    };
    match toml::from_str::<Settings>(&text) {
        Ok(settings) => match settings.auto_name.problem() {
            Some(why) => (Settings::default(), Some(unusable(&why))),
            None => (settings, None),
        },
        Err(e) => (Settings::default(), Some(unusable(e.message()))),
    }
}

/// Names the file alone: the full path would take most of the status area
/// beside the other startup notes (#149).
fn unusable(why: &str) -> String {
    format!("{FILE_NAME}: {why} - using defaults")
}

#[cfg(test)]
mod tests {
    use super::{load, AutoName, Settings, FILE_NAME};
    use std::path::{Path, PathBuf};

    fn dir_with(name: &str, contents: Option<&str>) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("palette-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        if let Some(text) = contents {
            std::fs::write(dir.join(FILE_NAME), text).unwrap();
        }
        dir
    }

    #[test]
    fn no_config_dir_means_defaults_and_no_message() {
        assert_eq!(load(None), (Settings::default(), None));
    }

    #[test]
    fn a_missing_file_means_defaults_and_no_message() {
        let dir = dir_with("missing", None);
        assert_eq!(load(Some(&dir)), (Settings::default(), None));
    }

    #[test]
    fn an_empty_file_means_defaults_and_no_message() {
        let dir = dir_with("empty", Some(""));
        assert_eq!(load(Some(&dir)), (Settings::default(), None));
    }

    #[test]
    fn icons_can_be_turned_off_and_on() {
        let off = dir_with("off", Some("icons = false\n"));
        assert_eq!(
            load(Some(&off)),
            (
                Settings {
                    icons: false,
                    ..Settings::default()
                },
                None
            )
        );
        let on = dir_with("on", Some("icons = true\n"));
        assert_eq!(load(Some(&on)), (Settings::default(), None));
    }

    #[test]
    fn a_misspelt_key_keeps_the_defaults_and_says_so() {
        let dir = dir_with("typo", Some("icon = false\n"));
        let (settings, why) = load(Some(&dir));
        assert_eq!(settings, Settings::default());
        let why = why.expect("a misspelt key is reported");
        assert!(why.contains("icon"), "{why}");
        assert!(why.contains(FILE_NAME), "{why}");
        assert!(!why.contains(dir.to_str().unwrap()), "{why}");
    }

    #[test]
    fn a_malformed_file_keeps_the_defaults_and_says_so() {
        for (name, text) in [("syntax", "icons = "), ("type", "icons = \"no\"\n")] {
            let dir = dir_with(name, Some(text));
            let (settings, why) = load(Some(&dir));
            assert_eq!(settings, Settings::default(), "{name}");
            assert!(why.unwrap().contains(FILE_NAME), "{name}");
        }
    }

    #[test]
    fn auto_name_defaults_when_absent() {
        let dir = dir_with("auto-absent", Some("icons = true\n"));
        let (settings, why) = load(Some(&dir));
        assert_eq!(why, None);
        assert_eq!(
            settings.auto_name,
            AutoName {
                model: "qwen3.5:9b".into(),
                base_url: "http://localhost:11434".into(),
                timeout_secs: super::TIMEOUT_SECS,
            }
        );
    }

    #[test]
    fn auto_name_is_read_from_its_table() {
        let dir = dir_with(
            "auto-set",
            Some("[auto_name]\nmodel = \"qwen3.6:35b\"\nbase_url = \"http://studio:11434\"\ntimeout_secs = 90\n"),
        );
        let (settings, why) = load(Some(&dir));
        assert_eq!(why, None);
        assert_eq!(
            settings.auto_name,
            AutoName {
                model: "qwen3.6:35b".into(),
                base_url: "http://studio:11434".into(),
                timeout_secs: 90,
            }
        );
    }

    #[test]
    fn a_partial_auto_name_table_keeps_the_other_defaults() {
        let dir = dir_with("auto-partial", Some("[auto_name]\nmodel = \"llama3\"\n"));
        let (settings, _) = load(Some(&dir));
        assert_eq!(
            settings.auto_name,
            AutoName {
                model: "llama3".into(),
                ..AutoName::default()
            }
        );
    }

    #[test]
    fn an_unusable_auto_name_value_keeps_the_defaults_and_says_why() {
        for (name, text, named) in [
            ("model", "[auto_name]\nmodel = \" \"\n", "model"),
            (
                "https",
                "[auto_name]\nbase_url = \"https://x\"\n",
                "base_url",
            ),
            (
                "bare",
                "[auto_name]\nbase_url = \"localhost:11434\"\n",
                "base_url",
            ),
            ("zero", "[auto_name]\ntimeout_secs = 0\n", "timeout_secs"),
            (
                "huge",
                "[auto_name]\ntimeout_secs = 9223372036854775807\n",
                "timeout_secs",
            ),
            ("negative", "[auto_name]\ntimeout_secs = -1\n", "-1"),
            ("typo", "[auto_name]\nmodle = \"x\"\n", "modle"),
        ] {
            let dir = dir_with(
                &format!("auto-{name}"),
                Some(&format!("icons = false\n{text}")),
            );
            let (settings, why) = load(Some(&dir));
            assert_eq!(settings, Settings::default(), "{name}");
            let why = why.unwrap_or_else(|| panic!("{name} is reported"));
            assert!(why.contains(named), "{name}: {why}");
            assert!(why.contains(FILE_NAME), "{name}: {why}");
        }
    }

    /// A read error other than absence — here the name is a directory — is not
    /// the ordinary case, so it is reported rather than read as "no file".
    #[test]
    fn an_unreadable_file_keeps_the_defaults_and_says_so() {
        let dir = dir_with("unreadable", None);
        std::fs::create_dir(dir.join(FILE_NAME)).unwrap();
        let (settings, why) = load(Some(Path::new(&dir)));
        assert_eq!(settings, Settings::default());
        assert!(why.unwrap().contains(FILE_NAME));
    }
}
