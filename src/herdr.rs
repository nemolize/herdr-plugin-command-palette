//! Dispatch into Herdr through $HERDR_BIN_PATH — the full CLI, JSON responses,
//! one process spawn per action (docs/design.md §3).
use std::process::Command as Proc;

use serde::Deserialize;

/// A row from one of the list APIs, reduced to what a candidate needs.
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

    /// The CLI reports API failures in its JSON body and still exits 0, so the
    /// body is what decides success — not the exit status.
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
        let args: Vec<String> = resolve.split_whitespace().map(str::to_owned).collect();
        let (collection, id_key) = match args.first().map(String::as_str) {
            Some("pane") => ("panes", "pane_id"),
            Some("tab") => ("tabs", "tab_id"),
            Some("workspace") => ("workspaces", "workspace_id"),
            other => {
                return Err(format!(
                    "unsupported resolve target: {}",
                    other.unwrap_or("")
                ))
            }
        };

        let result = self.call(&args)?;
        let rows = result
            .get(collection)
            .and_then(|v| v.as_array())
            .ok_or_else(|| format!("{resolve} returned no {collection}"))?;

        Ok(rows
            .iter()
            .filter_map(|row| {
                let id = row.get(id_key)?.as_str()?.to_string();
                let label = row
                    .get("label")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .or_else(|| {
                        row.get("terminal_title_stripped")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                    })
                    .unwrap_or(&id)
                    .to_string();
                let focused = row
                    .get("focused")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let label = if focused {
                    format!("{label} (current)")
                } else {
                    label
                };
                Some(Target { id, label })
            })
            .collect())
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
}
