//! The invocation context Herdr hands the pane in `HERDR_PLUGIN_CONTEXT_JSON`
//! (docs/design.md §8).
//!
//! This is the only route to the ids on herdr 0.8.2: the pane process receives
//! no `HERDR_PANE_ID` / `HERDR_TAB_ID` / `HERDR_WORKSPACE_ID` of its own, and
//! the ids here describe the pane the user was in when they opened the palette,
//! which is the one an entry should act on.
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
pub struct Context {
    #[serde(default)]
    pub focused_pane_id: Option<String>,
    #[serde(default)]
    pub tab_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

impl Context {
    pub fn from_env() -> Self {
        std::env::var("HERDR_PLUGIN_CONTEXT_JSON")
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    /// Which `contexts` value the palette is running under. The narrowest
    /// available id wins, so a pane-scoped entry is offered whenever there is a
    /// pane to act on.
    pub fn scope(&self) -> &'static str {
        if self.focused_pane_id.is_some() {
            "pane"
        } else if self.tab_id.is_some() {
            "tab"
        } else if self.workspace_id.is_some() {
            "workspace"
        } else {
            "global"
        }
    }

    pub fn lookup(&self, placeholder: &str) -> Option<&str> {
        match placeholder {
            "{pane}" => self.focused_pane_id.as_deref(),
            "{tab}" => self.tab_id.as_deref(),
            "{workspace}" => self.workspace_id.as_deref(),
            _ => None,
        }
    }

    /// An entry whose argv names a context id the current invocation lacks
    /// cannot run, so it is not offered rather than failing when picked.
    pub fn can_satisfy(&self, args: &[String]) -> bool {
        args.iter().all(|a| match a.as_str() {
            "{pane}" | "{tab}" | "{workspace}" => self.lookup(a).is_some(),
            _ => true,
        })
    }

    pub fn substitute(&self, args: &[String]) -> Vec<String> {
        args.iter()
            .map(|a| {
                self.lookup(a)
                    .map(str::to_owned)
                    .unwrap_or_else(|| a.clone())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Context;

    fn ctx(pane: Option<&str>, tab: Option<&str>, ws: Option<&str>) -> Context {
        Context {
            focused_pane_id: pane.map(str::to_owned),
            tab_id: tab.map(str::to_owned),
            workspace_id: ws.map(str::to_owned),
        }
    }

    #[test]
    fn scope_narrows_to_the_most_specific_id() {
        assert_eq!(
            ctx(Some("w1:p1"), Some("w1:t1"), Some("w1")).scope(),
            "pane"
        );
        assert_eq!(ctx(None, Some("w1:t1"), Some("w1")).scope(), "tab");
        assert_eq!(ctx(None, None, Some("w1")).scope(), "workspace");
        assert_eq!(ctx(None, None, None).scope(), "global");
    }

    #[test]
    fn substitutes_context_placeholders() {
        let c = ctx(Some("w1:p1"), Some("w1:t1"), None);
        let args = ["pane", "close", "{pane}"].map(str::to_owned).to_vec();
        assert_eq!(c.substitute(&args), vec!["pane", "close", "w1:p1"]);
    }

    #[test]
    fn an_entry_needing_a_missing_id_is_not_offered() {
        let c = ctx(None, Some("w1:t1"), None);
        let needs_pane = ["pane", "close", "{pane}"].map(str::to_owned).to_vec();
        let needs_tab = ["tab", "close", "{tab}"].map(str::to_owned).to_vec();
        assert!(!c.can_satisfy(&needs_pane));
        assert!(c.can_satisfy(&needs_tab));
    }

    #[test]
    fn an_entry_with_no_placeholder_is_always_satisfiable() {
        let args = ["tab", "create"].map(str::to_owned).to_vec();
        assert!(ctx(None, None, None).can_satisfy(&args));
    }

    #[test]
    fn a_missing_context_json_is_an_empty_context_not_an_error() {
        assert_eq!(Context::default().scope(), "global");
    }
}
