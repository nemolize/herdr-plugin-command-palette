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

        read_response(&out.stdout, &out.stderr)
    }

    /// `herdr --version` prints a plain "herdr X.Y.Z" line rather than JSON, so
    /// this is the one call that does not go through `call`.
    pub fn version(&self) -> Option<String> {
        let out = Proc::new(&self.bin).arg("--version").output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        text.split_whitespace().last().map(str::to_owned)
    }

    /// The config template herdr ships, whose `[keys]` block carries every
    /// action's default binding. Like `version`, it is plain text rather than
    /// JSON and so does not go through `call`.
    ///
    /// None on any failure: a keys column the palette cannot fill is a column
    /// it leaves empty, never a reason to refuse to open.
    pub fn default_config(&self) -> Option<String> {
        let out = Proc::new(&self.bin).arg("--default-config").output().ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8(out.stdout).ok()
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

    /// The current name of `id`, for seeding a rename. Empty when there is no
    /// name to edit, including when the lookup fails: an unseeded prompt still
    /// renames, so it is not worth refusing the stage over.
    pub fn current_label(&self, resolve: &str, id: &str) -> String {
        let (collection, id_key) = match resolve {
            "pane list" => ("panes", "pane_id"),
            "tab list" => ("tabs", "tab_id"),
            "workspace list" => ("workspaces", "workspace_id"),
            _ => return String::new(),
        };
        let args: Vec<String> = resolve.split_whitespace().map(str::to_owned).collect();
        let Ok(result) = self.call(&args) else {
            return String::new();
        };
        result
            .get(collection)
            .and_then(|v| v.as_array())
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.get(id_key).and_then(|v| v.as_str()) == Some(id))
            })
            .map(seed_from_row)
            .unwrap_or_default()
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

/// The value herdr returned, or the best message available for what went wrong.
/// Pure so the failure text is testable: it is the only thing the user sees when
/// a dispatch fails, and a herdr that answers with something other than the
/// envelope leaves this as the sole account of it.
fn read_response(stdout: &[u8], stderr: &[u8]) -> Result<serde_json::Value, String> {
    let body: Envelope = serde_json::from_slice(stdout).map_err(|_| {
        // Preferred over stdout because a herdr that failed before writing its
        // envelope says why on stderr, and stdout is then empty or a fragment.
        let stderr = String::from_utf8_lossy(stderr);
        let text = if stderr.trim().is_empty() {
            String::from_utf8_lossy(stdout).trim().to_string()
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

/// The name to seed a rename input with, or empty when there is none to edit.
///
/// An unnamed tab's `label` IS its `number` as a string, so seeding from `label`
/// alone would offer `1` and a bare Enter would apply it.
fn seed_from_row(row: &serde_json::Value) -> String {
    let label = row
        .get("label")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or_default();
    match row.get("number").and_then(|v| v.as_u64()) {
        Some(n) if label == n.to_string() => String::new(),
        _ => label.to_string(),
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
    use super::{read_response, seed_from_row, target_from_row, Herdr, PluginAction};
    use serde_json::json;

    /// The envelope's own error is what names a failure; the exit status is 1
    /// for an API error and 1 for a missing binary alike.
    #[test]
    fn an_api_error_surfaces_its_message() {
        let body = br#"{"error":{"message":"pane not found"}}"#;
        let err = read_response(body, b"").expect_err("an error body is a failure");
        assert_eq!(err, "pane not found");
    }

    #[test]
    fn a_result_body_is_returned() {
        let body = br#"{"result":{"panes":[]}}"#;
        let value = read_response(body, b"").expect("a result body is a success");
        assert!(value.get("panes").is_some(), "{value}");
    }

    /// A herdr that dies before writing its envelope explains itself on stderr,
    /// so that text is what the user needs rather than the empty stdout.
    #[test]
    fn non_json_output_falls_back_to_stderr() {
        let err = read_response(b"", b"error: unknown subcommand 'pane'\n")
            .expect_err("non-JSON is a failure");
        assert_eq!(err, "error: unknown subcommand 'pane'");
    }

    /// With nothing on stderr, whatever reached stdout is the only account of
    /// the failure — a usage line, say, printed where the envelope was due.
    #[test]
    fn non_json_output_falls_back_to_stdout_when_stderr_is_silent() {
        let err =
            read_response(b"Usage: herdr <command>\n", b"").expect_err("non-JSON is a failure");
        assert_eq!(err, "Usage: herdr <command>");
    }

    /// Both streams empty is its own case: without it the user is shown an
    /// empty string, which reads as the palette having done nothing.
    #[test]
    fn a_silent_failure_still_says_something() {
        let err = read_response(b"", b"").expect_err("no output is a failure");
        assert_eq!(err, "herdr returned no output");
    }

    /// A well-formed envelope carrying neither half is not a success: taking it
    /// as one would let a dispatch that did nothing report as having run.
    #[test]
    fn an_envelope_with_neither_result_nor_error_is_a_failure() {
        let err = read_response(b"{}", b"").expect_err("an empty envelope is a failure");
        assert_eq!(err, "herdr returned no result");
    }

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

    /// An unnamed tab's `label` IS its number, so seeding from `label` alone
    /// would offer `1` as the new name and a bare Enter would apply it.
    #[test]
    fn an_unnamed_row_seeds_nothing() {
        let row = json!({"tab_id": "w5A:t1", "label": "1", "number": 1});
        assert_eq!(seed_from_row(&row), "");
    }

    #[test]
    fn a_named_row_seeds_its_name() {
        let row = json!({"tab_id": "w5E:t1", "label": "release notes", "number": 1});
        assert_eq!(seed_from_row(&row), "release notes");
    }

    /// A name that merely looks numeric is still a name the user chose — only
    /// the row's OWN number is the placeholder.
    #[test]
    fn a_numeric_name_that_is_not_the_row_number_is_kept() {
        let row = json!({"tab_id": "w5E:t1", "label": "2024", "number": 1});
        assert_eq!(seed_from_row(&row), "2024");
    }

    /// An unnamed pane reports no `label` at all — its terminal title is not a
    /// name the user set, so there is nothing to edit.
    #[test]
    fn a_row_without_a_label_seeds_nothing() {
        let row = json!({"pane_id": "w5A:p1", "terminal_title_stripped": "Claude Code"});
        assert_eq!(seed_from_row(&row), "");
    }

    /// A named pane DOES report a label, and carries no `number` to mistake it
    /// for — so the number rule must not swallow it.
    #[test]
    fn a_named_pane_seeds_its_name() {
        let row = json!({"pane_id": "w5A:p1", "label": "editor"});
        assert_eq!(seed_from_row(&row), "editor");
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
