//! What the model reads to name a workspace. Latency is almost all prompt
//! prefill, so this is kept to the panes' own one-line descriptions plus the
//! recent output of the pane the palette was opened from.
use std::process::Command as Proc;

use serde_json::Value;

use crate::herdr::Herdr;

pub const FOCUSED_LINES: u32 = 120;

#[derive(Debug, PartialEq)]
pub struct PaneBrief {
    pub title: String,
    pub cwd: String,
    pub branch: Option<String>,
}

pub fn gather(
    herdr: &Herdr,
    workspace: &str,
    focused_pane: Option<&str>,
) -> Result<String, String> {
    let panes: Vec<PaneBrief> = herdr
        .workspace_panes(workspace)
        .map_err(|e| format!("pane list: {e}"))?
        .iter()
        .map(|row| {
            let (title, cwd) = title_and_cwd(row);
            let branch = branch_at(&cwd);
            PaneBrief { title, cwd, branch }
        })
        .collect();
    let output = match focused_pane {
        Some(pane) => Some(
            herdr
                .read_pane(pane, FOCUSED_LINES)
                .map_err(|e| format!("pane read: {e}"))?,
        ),
        None => None,
    };
    Ok(prompt(&panes, output.as_deref()))
}

fn title_and_cwd(row: &Value) -> (String, String) {
    let field = |key: &str| {
        row.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    (field("terminal_title_stripped"), field("foreground_cwd"))
}

/// None when `cwd` is not in a repository, is on a detached HEAD, or git is
/// not installed — the model is told "none" in all three cases.
fn branch_at(cwd: &str) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    let out = Proc::new("git")
        .args(["-C", cwd, "branch", "--show-current"])
        .output()
        .ok()?;
    let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !branch.is_empty()).then_some(branch)
}

pub fn prompt(panes: &[PaneBrief], focused_output: Option<&str>) -> String {
    let mut text = String::from("Panes in this workspace:\n");
    for pane in panes {
        text.push_str(&format!(
            "- title: {}; cwd: {}; git branch: {}\n",
            or_none(&pane.title),
            or_none(&pane.cwd),
            pane.branch.as_deref().unwrap_or("none"),
        ));
    }
    if let Some(output) = focused_output {
        text.push_str(&format!(
            "\nLast {FOCUSED_LINES} lines of the focused pane:\n{}\n",
            output.trim_end()
        ));
    }
    text
}

fn or_none(s: &str) -> &str {
    if s.trim().is_empty() {
        "none"
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_pane_row_gives_its_stripped_title_and_foreground_cwd() {
        let row = json!({
            "terminal_title": "* build",
            "terminal_title_stripped": "build",
            "cwd": "/start",
            "foreground_cwd": "/now",
        });
        assert_eq!(title_and_cwd(&row), ("build".into(), "/now".into()));
        assert_eq!(title_and_cwd(&json!({})), (String::new(), String::new()));
    }

    #[test]
    fn the_prompt_lists_every_pane_then_the_focused_output() {
        let panes = [
            PaneBrief {
                title: "cargo test".into(),
                cwd: "/src/palette".into(),
                branch: Some("feat-240".into()),
            },
            PaneBrief {
                title: String::new(),
                cwd: "/tmp".into(),
                branch: None,
            },
        ];
        assert_eq!(
            prompt(&panes, Some("line one\nline two\n\n")),
            "Panes in this workspace:\n\
             - title: cargo test; cwd: /src/palette; git branch: feat-240\n\
             - title: none; cwd: /tmp; git branch: none\n\
             \n\
             Last 120 lines of the focused pane:\n\
             line one\nline two\n"
        );
    }

    #[test]
    fn without_a_focused_pane_only_the_panes_are_sent() {
        let text = prompt(&[], None);
        assert_eq!(text, "Panes in this workspace:\n");
    }

    #[test]
    fn a_directory_git_cannot_enter_has_no_branch() {
        assert_eq!(branch_at("/nonexistent-palette-dir"), None);
        assert_eq!(branch_at(""), None);
    }

    #[test]
    fn a_repository_reports_its_checked_out_branch() {
        let cwd = env!("CARGO_MANIFEST_DIR");
        let expected = Proc::new("git")
            .args(["-C", cwd, "branch", "--show-current"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let expected = (!expected.is_empty()).then_some(expected);
        assert_eq!(branch_at(cwd), expected);
    }
}
