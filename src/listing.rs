//! Reading the list APIs' rows into what the palette shows and substitutes.
//! Pure, so the label rules are testable without a herdr binary — `herdr.rs`
//! holds the only process seam.

/// A row from one of the list APIs, reduced to what a candidate needs.
#[derive(Debug)]
pub struct Target {
    pub id: String,
    pub label: String,
}

/// The CLI addresses a worktree by path (`open`) or by the workspace it is open
/// in (`remove`), so the flag before `{}` names the row column that fills it.
/// `Workspace` offers only linked worktrees: `remove` refuses the main checkout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeKey {
    Path,
    Workspace,
}

impl WorktreeKey {
    pub fn for_args(args: &[String]) -> Option<Self> {
        let at = args.iter().position(|a| a == "{}")?;
        match args.get(at.checked_sub(1)?)?.as_str() {
            "--path" => Some(Self::Path),
            "--workspace" => Some(Self::Workspace),
            _ => None,
        }
    }
}

/// The name to seed a rename input with, or empty when there is none to edit.
///
/// An unnamed tab's `label` IS its `number` as a string, so seeding from `label`
/// alone would offer `1` and a bare Enter would apply it.
pub fn seed_from_row(row: &serde_json::Value) -> String {
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

/// Unscoped, `worktree list` reads the server's focused workspace, which while
/// the palette is up is not reliably the one it was opened from (`--current`).
pub fn worktree_list_args(workspace: &str) -> Vec<String> {
    ["worktree", "list", "--workspace", workspace]
        .map(str::to_owned)
        .to_vec()
}

pub fn source_repo_root(listing: &serde_json::Value) -> Result<String, String> {
    listing
        .pointer("/source/repo_root")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| "worktree list named no repository root".to_string())
}

/// The workspaces holding a worktree herdr created, the only ones `worktree
/// remove` accepts: a `git worktree add` checkout opened as a plain workspace
/// lists identically, and only its `workspace list` row tells it apart.
pub fn managed_worktree_workspaces(workspace_list: &serde_json::Value) -> Vec<String> {
    workspace_list
        .get("workspaces")
        .and_then(|v| v.as_array())
        .map(|rows| {
            rows.iter()
                .filter(|row| {
                    row.pointer("/worktree/is_linked_worktree")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                })
                .filter_map(|row| row.get("workspace_id")?.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// herdr refuses to open the bare repository and a prunable worktree, whose
/// directory is gone; `remove` still clears a prunable one.
fn herdr_can_open(row: &serde_json::Value) -> bool {
    !["is_bare", "is_prunable"]
        .iter()
        .any(|k| row.get(k).and_then(|v| v.as_bool()).unwrap_or(false))
}

/// Every row's `label` is the repository name, so the branch tells rows apart.
/// `Workspace` keeps only rows open in one of `managed`.
pub fn worktree_target(
    row: &serde_json::Value,
    key: WorktreeKey,
    managed: &[String],
) -> Option<Target> {
    let path = row.get("path")?.as_str()?;
    let open = row.get("open_workspace_id").and_then(|v| v.as_str());
    let id = match key {
        WorktreeKey::Path if !herdr_can_open(row) => return None,
        WorktreeKey::Path => path,
        WorktreeKey::Workspace => open.filter(|w| managed.iter().any(|m| m == w))?,
    };
    let mut label = row
        .get("branch")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string();
    if open.is_some() {
        label.push_str(" (open)");
    }
    Some(Target {
        id: id.to_string(),
        label,
    })
}

/// Builds one candidate row from a pane, tab or workspace listing.
pub fn target_from_row(
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
    use super::{
        managed_worktree_workspaces, seed_from_row, source_repo_root, target_from_row,
        worktree_list_args, worktree_target, WorktreeKey,
    };
    use serde_json::json;

    #[test]
    fn a_worktree_placeholder_is_filled_from_the_column_its_flag_names() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            WorktreeKey::for_args(&args(&["worktree", "open", "--path", "{}"])),
            Some(WorktreeKey::Path)
        );
        assert_eq!(
            WorktreeKey::for_args(&args(&["worktree", "remove", "--workspace", "{}"])),
            Some(WorktreeKey::Workspace)
        );
        assert_eq!(
            WorktreeKey::for_args(&args(&["worktree", "open", "--branch", "{}"])),
            None
        );
        assert_eq!(WorktreeKey::for_args(&args(&["{}"])), None);
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
    #[test]
    fn the_worktree_listing_is_scoped_to_the_context_workspace() {
        assert_eq!(
            worktree_list_args("w1"),
            vec!["worktree", "list", "--workspace", "w1"]
        );
    }

    #[test]
    fn the_repository_root_comes_from_the_listing_source() {
        let listing = json!({"source": {"repo_root": "/src/repo", "source_workspace_id": "w1"},
                             "worktrees": []});
        assert_eq!(source_repo_root(&listing), Ok("/src/repo".to_string()));
        assert!(source_repo_root(&json!({"worktrees": []})).is_err());
    }

    fn worktree_rows() -> [serde_json::Value; 4] {
        [
            json!({"branch": "main", "is_linked_worktree": false, "label": "repo",
                   "open_workspace_id": "w1", "path": "/src/repo"}),
            json!({"branch": "feat-x", "is_linked_worktree": true, "label": "repo",
                   "open_workspace_id": "w2", "path": "/wt/repo/feat-x"}),
            json!({"is_detached": true, "is_linked_worktree": true, "label": "repo",
                   "path": "/wt/repo/detached"}),
            json!({"branch": "manual", "is_linked_worktree": true, "label": "repo",
                   "open_workspace_id": "w3", "path": "/src/manual"}),
        ]
    }

    fn unopenable_rows() -> [serde_json::Value; 2] {
        [
            json!({"is_bare": true, "is_linked_worktree": false, "label": "repo",
                   "path": "/src/repo.git"}),
            json!({"branch": "gone", "is_linked_worktree": true, "is_prunable": true,
                   "label": "repo", "open_workspace_id": "w2", "path": "/wt/repo/gone"}),
        ]
    }

    /// w3 holds a `git worktree add` checkout herdr did not create.
    fn managed() -> Vec<String> {
        managed_worktree_workspaces(&json!({"workspaces": [
            {"workspace_id": "w1", "worktree": {"is_linked_worktree": false}},
            {"workspace_id": "w2", "worktree": {"is_linked_worktree": true}},
            {"workspace_id": "w3"},
        ]}))
    }

    #[test]
    fn worktree_rows_sharing_a_label_are_told_apart_by_branch() {
        let labels: Vec<String> = worktree_rows()
            .iter()
            .filter_map(|r| worktree_target(r, WorktreeKey::Path, &[]))
            .map(|t| t.label)
            .collect();
        assert_eq!(
            labels,
            vec![
                "main (open)",
                "feat-x (open)",
                "/wt/repo/detached",
                "manual (open)"
            ]
        );
    }

    #[test]
    fn opening_offers_every_worktree_by_path() {
        let ids: Vec<String> = worktree_rows()
            .iter()
            .filter_map(|r| worktree_target(r, WorktreeKey::Path, &[]))
            .map(|t| t.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                "/src/repo",
                "/wt/repo/feat-x",
                "/wt/repo/detached",
                "/src/manual"
            ]
        );
    }

    #[test]
    fn removing_offers_only_worktrees_herdr_manages_by_workspace() {
        let managed = managed();
        let ids: Vec<String> = worktree_rows()
            .iter()
            .filter_map(|r| worktree_target(r, WorktreeKey::Workspace, &managed))
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, vec!["w2"]);
    }

    #[test]
    fn opening_offers_neither_a_bare_nor_a_prunable_row() {
        for row in &unopenable_rows() {
            assert!(worktree_target(row, WorktreeKey::Path, &[]).is_none());
        }
    }

    #[test]
    fn removing_still_offers_a_prunable_worktree_herdr_manages() {
        let ids: Vec<String> = unopenable_rows()
            .iter()
            .filter_map(|r| worktree_target(r, WorktreeKey::Workspace, &managed()))
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, vec!["w2"]);
    }
}
