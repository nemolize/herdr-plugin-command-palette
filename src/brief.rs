//! What the model reads to name a workspace, a tab or a pane. Latency is almost
//! all prompt prefill, so this is kept to the panes' own one-line descriptions
//! plus the recent output of one pane.
use std::process::Command as Proc;

use serde_json::Value;

use crate::herdr::Herdr;
use crate::ollama::NAME_CAP;

pub const FOCUSED_LINES: u32 = 120;

#[derive(Debug, Clone, PartialEq)]
pub enum Subject {
    Workspace(String),
    Tab(String),
    Pane(String),
}

impl Subject {
    /// The subject a rename entry acts on; None for every other entry, and for
    /// a rename whose id the context could not fill.
    pub fn of_rename(args: &[String]) -> Option<Self> {
        let [subject, verb, id, ..] = args else {
            return None;
        };
        if verb != "rename" || id.starts_with('{') {
            return None;
        }
        let id = id.clone();
        match subject.as_str() {
            "workspace" => Some(Self::Workspace(id)),
            "tab" => Some(Self::Tab(id)),
            "pane" => Some(Self::Pane(id)),
            _ => None,
        }
    }

    fn noun(&self) -> &'static str {
        match self {
            Self::Workspace(_) => "workspace",
            Self::Tab(_) => "tab",
            Self::Pane(_) => "pane",
        }
    }

    pub fn system_prompt(&self) -> String {
        let noun = self.noun();
        let read = match self {
            Self::Pane(_) => "pane",
            _ => "panes",
        };
        format!(
            "You name terminal {noun}s. Read the {read} below and reply with one label of at \
most {NAME_CAP} characters for what this {noun} is about. Prefer the concrete subject (a \
project, feature, or task) over generic words such as terminal, dev, or work. Write the label \
in the language the human uses in the input. Reply with the label only: one line, no quotes, \
no explanation."
        )
    }
}

#[derive(Debug, PartialEq)]
pub struct PaneBrief {
    pub title: String,
    pub cwd: String,
    pub branch: Option<String>,
}

/// `focused_pane` is the pane the palette was opened from, passed only when it
/// belongs to `subject`; a pane subject reads its own output instead.
pub fn gather(
    herdr: &Herdr,
    subject: &Subject,
    focused_pane: Option<&str>,
) -> Result<String, String> {
    let rows = match subject {
        Subject::Workspace(id) => herdr.workspace_panes(id),
        Subject::Tab(id) => herdr.tab_panes(id),
        Subject::Pane(id) => herdr.pane(id).map(|row| vec![row]),
    }
    .map_err(|e| format!("pane list: {e}"))?;
    let panes: Vec<PaneBrief> = rows
        .iter()
        .map(|row| {
            let (title, cwd) = title_and_cwd(row);
            let branch = branch_at(&cwd);
            PaneBrief { title, cwd, branch }
        })
        .collect();
    let read = match subject {
        Subject::Pane(id) => Some(id.as_str()),
        _ => focused_pane,
    };
    let output = match read {
        Some(pane) => Some(
            herdr
                .read_pane(pane, FOCUSED_LINES)
                .map_err(|e| format!("pane read: {e}"))?,
        ),
        None => None,
    };
    Ok(prompt(subject, &panes, output.as_deref()))
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
/// not installed.
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

pub fn prompt(subject: &Subject, panes: &[PaneBrief], output: Option<&str>) -> String {
    let (heading, output_of) = match subject {
        Subject::Pane(_) => ("This pane:", "this pane"),
        Subject::Tab(_) => ("Panes in this tab:", "the focused pane"),
        Subject::Workspace(_) => ("Panes in this workspace:", "the focused pane"),
    };
    let mut text = format!("{heading}\n");
    for pane in panes {
        text.push_str(&format!(
            "- title: {}; cwd: {}; git branch: {}\n",
            or_none(&pane.title),
            or_none(&pane.cwd),
            pane.branch.as_deref().unwrap_or("none"),
        ));
    }
    if let Some(output) = output {
        text.push_str(&format!(
            "\nLast {FOCUSED_LINES} lines of {output_of}:\n{}\n",
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

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn each_rename_names_its_own_subject() {
        let of = |a: &[&str]| Subject::of_rename(&args(a));
        assert_eq!(
            of(&["workspace", "rename", "w3Y", "{text}"]),
            Some(Subject::Workspace("w3Y".into()))
        );
        assert_eq!(
            of(&["tab", "rename", "w3Y:t1", "{text}"]),
            Some(Subject::Tab("w3Y:t1".into()))
        );
        assert_eq!(
            of(&["pane", "rename", "w3Y:p1", "{text}"]),
            Some(Subject::Pane("w3Y:p1".into()))
        );
    }

    #[test]
    fn anything_but_a_rename_with_a_filled_id_names_nothing() {
        let of = |a: &[&str]| Subject::of_rename(&args(a));
        assert_eq!(of(&["tab", "close", "w3Y:t1"]), None);
        assert_eq!(of(&["pane", "rename", "{text}"]), None, "no id to describe");
        assert_eq!(of(&["tab", "rename", "{tab}", "{text}"]), None);
        assert_eq!(of(&["worktree", "rename", "x", "{text}"]), None);
        assert_eq!(of(&["tab", "rename"]), None);
    }

    #[test]
    fn the_system_prompt_asks_for_the_subject_being_named() {
        let workspace = Subject::Workspace("w".into()).system_prompt();
        assert!(workspace.starts_with("You name terminal workspaces. Read the panes below"));
        assert!(workspace.contains("what this workspace is about"));
        assert!(workspace.contains("at most 20 characters"));

        let tab = Subject::Tab("t".into()).system_prompt();
        assert!(tab.starts_with("You name terminal tabs. Read the panes below"));
        assert!(tab.contains("what this tab is about"));

        let pane = Subject::Pane("p".into()).system_prompt();
        assert!(pane.starts_with("You name terminal panes. Read the pane below"));
        assert!(pane.contains("what this pane is about"));
    }

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

    fn two_panes() -> [PaneBrief; 2] {
        [
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
        ]
    }

    #[test]
    fn the_prompt_lists_every_pane_then_the_focused_output() {
        assert_eq!(
            prompt(
                &Subject::Workspace("w".into()),
                &two_panes(),
                Some("line one\nline two\n\n")
            ),
            "Panes in this workspace:\n\
             - title: cargo test; cwd: /src/palette; git branch: feat-240\n\
             - title: none; cwd: /tmp; git branch: none\n\
             \n\
             Last 120 lines of the focused pane:\n\
             line one\nline two\n"
        );
    }

    #[test]
    fn a_tab_prompt_says_it_lists_the_tabs_panes() {
        assert_eq!(
            prompt(&Subject::Tab("t".into()), &two_panes(), Some("out")),
            "Panes in this tab:\n\
             - title: cargo test; cwd: /src/palette; git branch: feat-240\n\
             - title: none; cwd: /tmp; git branch: none\n\
             \n\
             Last 120 lines of the focused pane:\n\
             out\n"
        );
    }

    #[test]
    fn a_pane_prompt_describes_that_pane_and_its_own_output() {
        let [pane, _] = two_panes();
        assert_eq!(
            prompt(&Subject::Pane("p".into()), &[pane], Some("out")),
            "This pane:\n\
             - title: cargo test; cwd: /src/palette; git branch: feat-240\n\
             \n\
             Last 120 lines of this pane:\n\
             out\n"
        );
    }

    #[test]
    fn without_a_focused_pane_only_the_panes_are_sent() {
        let text = prompt(&Subject::Workspace("w".into()), &[], None);
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
