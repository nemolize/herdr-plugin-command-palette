//! Switches kept apart from `catalog.toml`, because a user catalog replaces the
//! shipped one wholesale and a switch there would cost a copy of all of it.
use std::io::ErrorKind;
use std::path::Path;

use serde::Deserialize;

pub const FILE_NAME: &str = "settings.toml";

#[derive(Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub icons: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { icons: true }
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
    match toml::from_str(&text) {
        Ok(settings) => (settings, None),
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
    use super::{load, Settings, FILE_NAME};
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
        assert_eq!(load(None), (Settings { icons: true }, None));
    }

    #[test]
    fn a_missing_file_means_defaults_and_no_message() {
        let dir = dir_with("missing", None);
        assert_eq!(load(Some(&dir)), (Settings { icons: true }, None));
    }

    #[test]
    fn an_empty_file_means_defaults_and_no_message() {
        let dir = dir_with("empty", Some(""));
        assert_eq!(load(Some(&dir)), (Settings { icons: true }, None));
    }

    #[test]
    fn icons_can_be_turned_off_and_on() {
        let off = dir_with("off", Some("icons = false\n"));
        assert_eq!(load(Some(&off)), (Settings { icons: false }, None));
        let on = dir_with("on", Some("icons = true\n"));
        assert_eq!(load(Some(&on)), (Settings { icons: true }, None));
    }

    #[test]
    fn a_misspelt_key_keeps_the_defaults_and_says_so() {
        let dir = dir_with("typo", Some("icon = false\n"));
        let (settings, why) = load(Some(&dir));
        assert_eq!(settings, Settings { icons: true });
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
            assert_eq!(settings, Settings { icons: true }, "{name}");
            assert!(why.unwrap().contains(FILE_NAME), "{name}");
        }
    }

    /// A read error other than absence — here the name is a directory — is not
    /// the ordinary case, so it is reported rather than read as "no file".
    #[test]
    fn an_unreadable_file_keeps_the_defaults_and_says_so() {
        let dir = dir_with("unreadable", None);
        std::fs::create_dir(dir.join(FILE_NAME)).unwrap();
        let (settings, why) = load(Some(Path::new(&dir)));
        assert_eq!(settings, Settings { icons: true });
        assert!(why.unwrap().contains(FILE_NAME));
    }
}
