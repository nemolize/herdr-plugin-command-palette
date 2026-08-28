//! Dispatch into Herdr through $HERDR_BIN_PATH — the full CLI, JSON responses,
//! one process spawn per action (docs/design.md §3).
use std::process::Command as Proc;

use serde::Deserialize;

/// A row from one of the list APIs, reduced to what a candidate needs.
#[derive(Debug)]
pub struct Target {
    pub id: String,
    pub label: String,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<ApiError>,
}

#[derive(Deserialize)]
struct ApiError {
    message: String,
}

pub struct Herdr {
    bin: String,
}

impl Herdr {
    pub fn new(bin: String) -> Self {
        Self { bin }
    }

    /// The CLI reports an API failure in its JSON body, and the body is what
    /// names it — the exit status is 1 for an API error and 1 for a missing
    /// binary alike, so it cannot tell the two apart on its own.
    fn call(&self, args: &[String]) -> Result<serde_json::Value, String> {
        let out = Proc::new(&self.bin)
            .args(args)
            .output()
            .map_err(|e| format!("could not run {}: {e}", self.bin))?;

        let body: Envelope = serde_json::from_slice(&out.stdout).map_err(|_| {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let text = if stderr.trim().is_empty() {
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            } else {
                stderr.trim().to_string()
            };
            if text.is_empty() {
                "herdr returned no output".to_string()
            } else {
                text
            }
        })?;

        if let Some(err) = body.error {
            return Err(err.message);
        }
        body.result
            .ok_or_else(|| "herdr returned no result".to_string())
    }

    /// `herdr --version` prints a plain "herdr X.Y.Z" line rather than JSON, so
    /// this is the one call that does not go through `call`.
    pub fn version(&self) -> Option<String> {
        let out = Proc::new(&self.bin).arg("--version").output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.split_whitespace().last().map(str::to_owned)
    }

    /// Runs a catalog entry. Surfaces a failure verbatim so a drifted entry
    /// reports itself the first time it is used rather than silently doing
    /// nothing (§4) — the caller names the command id alongside this.
    pub fn dispatch(&self, args: &[String]) -> Result<(), String> {
        self.call(args).map(|_| ())
    }

    /// Resolves a `resolve` key into candidate rows. The three list APIs all
    /// carry an id, a label and a `focused` flag, so one reader covers them.
    pub fn targets(&self, resolve: &str) -> Result<Vec<Target>, String> {
        // Matched whole, never by leading token: a catalog is user-replaceable
        // (`catalog::locate`), and this string is about to become argv. Reading
        // only the first word would let `workspace close` run a state-mutating
        // command at pick time wearing the name of a listing.
        let (collection, id_key) = match resolve {
            "pane list" => ("panes", "pane_id"),
            "tab list" => ("tabs", "tab_id"),
            "workspace list" => ("workspaces", "workspace_id"),
            other => return Err(format!("unsupported resolve target: {other}")),
        };
        let args: Vec<String> = resolve.split_whitespace().map(str::to_owned).collect();

        let result = self.call(&args)?;
        let rows = result
            .get(collection)
            .and_then(|v| v.as_array())
            .ok_or_else(|| format!("{resolve} returned no {collection}"))?;

        // A tab's own label is its per-workspace NUMBER, so on a real session
        // every tab is called "1" and the rows are indistinguishable. The
        // workspace name is what tells them apart, and tab rows carry the id to
        // look it up with.
        let workspaces = if collection == "tabs" {
            self.workspace_labels().unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(rows
            .iter()
            .filter_map(|row| target_from_row(row, id_key, &workspaces))
            .collect())
    }

    /// (workspace_id, label) pairs, for qualifying tab rows.
    fn workspace_labels(&self) -> Result<Vec<(String, String)>, String> {
        let args = ["workspace", "list"].map(str::to_owned).to_vec();
        let result = self.call(&args)?;
        Ok(result
            .get("workspaces")
            .and_then(|v| v.as_array())
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| {
                        let id = row.get("workspace_id")?.as_str()?;
                        let label = row.get("label").and_then(|v| v.as_str()).unwrap_or(id);
                        Some((id.to_string(), label.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Plugin actions merge into the same list, so one search covers both
    /// built-ins and other plugins (§4).
    pub fn plugin_actions(&self) -> Result<Vec<PluginAction>, String> {
        let args = ["plugin", "action", "list"].map(str::to_owned).to_vec();
        let result = self.call(&args)?;
        let rows = result
            .get("actions")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "plugin action list returned no actions".to_string())?;

        Ok(rows
            .iter()
            .filter_map(|row| {
                let plugin_id = row.get("plugin_id")?.as_str()?.to_string();
                let action_id = row.get("action_id")?.as_str()?.to_string();
                let title = row
                    .get("title")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&action_id)
                    .to_string();
                let platforms = row
                    .get("platforms")
                    .and_then(|v| v.as_array())
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|c| c.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let contexts = row
                    .get("contexts")
                    .and_then(|v| v.as_array())
                    .map(|rows| {
                        rows.iter()
                            .filter_map(|c| c.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                Some(PluginAction {
                    plugin_id,
                    action_id,
                    title,
                    contexts,
                    platforms,
                })
            })
            .collect())
    }
}

pub struct PluginAction {
    pub plugin_id: String,
    pub action_id: String,
    pub title: String,
    pub contexts: Vec<String>,
    pub platforms: Vec<String>,
}

impl PluginAction {
    /// An action declared for other platforms cannot run here. The listing
    /// returns every platform's rows, and a plugin shipping per-platform
    /// variants gives them the SAME title — so without this the palette shows
    /// duplicate rows that differ only in which one works.
    pub fn runs_on(&self, platform: &str) -> bool {
        self.platforms.is_empty() || self.platforms.iter().any(|p| p == platform)
    }
}

/// What `platforms` calls the host this binary was built for.
pub fn current_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        // Termux included: it travels inside `linux`, which is the whole reason
        // the manifest cannot declare it (docs/design.md §2).
        "linux"
    }
}

/// Builds one candidate row. Pure so the label rules are testable without a
/// herdr binary — `Herdr::call` stays the only process seam.
fn target_from_row(
    row: &serde_json::Value,
    id_key: &str,
    workspaces: &[(String, String)],
) -> Option<Target> {
    let id = row.get(id_key)?.as_str()?.to_string();
    let own = row
        .get("label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            row.get("terminal_title_stripped")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or(&id);

    let mut label = match row.get("workspace_id").and_then(|v| v.as_str()) {
        Some(ws) if !workspaces.is_empty() => workspaces
            .iter()
            .find(|(wid, _)| wid == ws)
            .map(|(_, name)| format!("{name} · {own}"))
            .unwrap_or_else(|| own.to_string()),
        _ => own.to_string(),
    };
    if row
        .get("focused")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        label.push_str(" (current)");
    }
    Some(Target { id, label })
}

#[cfg(test)]
mod tests {
    use super::{target_from_row, Herdr, PluginAction};
    use serde_json::json;

    fn workspaces() -> Vec<(String, String)> {
        vec![
            ("w46".to_string(), "wevox-front".to_string()),
            ("w3Y".to_string(), "command-palette".to_string()),
        ]
    }

    #[test]
    fn tab_rows_are_qualified_by_their_workspace() {
        // Every tab on a real session is labelled with its per-workspace
        // number, so without the workspace name these rows are all "1".
        let a = json!({"tab_id": "w46:t1", "label": "1", "workspace_id": "w46"});
        let b = json!({"tab_id": "w3Y:t1", "label": "1", "workspace_id": "w3Y"});
        let a = target_from_row(&a, "tab_id", &workspaces()).unwrap();
        let b = target_from_row(&b, "tab_id", &workspaces()).unwrap();
        assert_eq!(a.label, "wevox-front · 1");
        assert_eq!(b.label, "command-palette · 1");
        assert_ne!(a.label, b.label);
    }

    #[test]
    fn the_focused_row_says_so() {
        let row = json!({"tab_id": "w46:t1", "label": "1", "workspace_id": "w46", "focused": true});
        let t = target_from_row(&row, "tab_id", &workspaces()).unwrap();
        assert_eq!(t.label, "wevox-front · 1 (current)");
    }

    #[test]
    fn an_unknown_workspace_falls_back_to_the_bare_label() {
        let row = json!({"tab_id": "w99:t1", "label": "1", "workspace_id": "w99"});
        let t = target_from_row(&row, "tab_id", &workspaces()).unwrap();
        assert_eq!(t.label, "1");
    }

    #[test]
    fn a_pane_row_falls_back_to_its_terminal_title() {
        let row = json!({"pane_id": "w46:p1", "terminal_title_stripped": "Claude Code"});
        let t = target_from_row(&row, "pane_id", &[]).unwrap();
        assert_eq!(t.label, "Claude Code");
    }

    #[test]
    fn a_row_with_no_label_at_all_shows_its_id() {
        let row = json!({"workspace_id": "w46"});
        let t = target_from_row(&row, "workspace_id", &[]).unwrap();
        assert_eq!(t.label, "w46");
    }

    #[test]
    fn a_row_missing_its_id_is_skipped() {
        let row = json!({"label": "orphan"});
        assert!(target_from_row(&row, "tab_id", &[]).is_none());
    }

    fn action(platforms: &[&str]) -> PluginAction {
        PluginAction {
            plugin_id: "p".into(),
            action_id: "a".into(),
            title: "t".into(),
            contexts: vec![],
            platforms: platforms.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn an_action_for_another_platform_is_not_offered() {
        // Observed live: a plugin ships windows twins whose titles match their
        // unix ones exactly, so an unfiltered list shows duplicate rows.
        assert!(!action(&["windows"]).runs_on("macos"));
        assert!(action(&["linux", "macos"]).runs_on("macos"));
    }

    #[test]
    fn an_action_declaring_no_platforms_runs_anywhere() {
        assert!(action(&[]).runs_on("macos"));
        assert!(action(&[]).runs_on("linux"));
    }

    /// A catalog is user-replaceable, so `resolve` is untrusted input that
    /// becomes argv. Anything but the three listings must be refused before it
    /// can run — `targets` returns the rejection without spawning a process, so
    /// the bin path is never reached.
    #[test]
    fn rejects_a_resolve_that_is_not_one_of_the_three_listings() {
        let herdr = Herdr::new("/nonexistent-herdr-binary".to_string());
        for hostile in [
            "workspace close",
            "tab close",
            "pane close",
            "tab list --extra",
            "list",
            "",
        ] {
            let err = herdr.targets(hostile).expect_err(hostile);
            assert!(
                err.starts_with("unsupported resolve target"),
                "{hostile} was not refused: {err}"
            );
        }
    }
}
