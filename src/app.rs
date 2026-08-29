//! Candidate list and selection state, independent of how it is drawn.
use ratatui::widgets::ListState;

use crate::catalog::Command;
use crate::frecency::Frecency;
use crate::fuzzy;
use crate::herdr::{PluginAction, Target};

pub enum Kind {
    /// A catalog entry, runnable as written.
    Command(Command),
    /// Another plugin's action, invoked through `herdr plugin action invoke`.
    Action(PluginAction),
}

pub struct Candidate {
    pub id: String,
    pub title: String,
    pub kind: Kind,
}

impl Candidate {
    pub fn from_command(c: Command) -> Self {
        Self {
            id: c.id.clone(),
            title: c.title.clone(),
            kind: Kind::Command(c),
        }
    }

    pub fn from_action(a: PluginAction) -> Self {
        Self {
            id: format!("{}.{}", a.plugin_id, a.action_id),
            title: a.title.clone(),
            kind: Kind::Action(a),
        }
    }
}

/// What the palette is asking for right now. Picking an entry that needs a
/// target does not run it — it swaps the list for that entry's candidates, so
/// one palette covers both halves of the choice.
pub enum Stage {
    Commands,
    Targets {
        command: Command,
        targets: Vec<Target>,
    },
}

/// What the event loop should do after a keypress.
pub enum Step {
    Continue,
    Cancel,
    /// Fetch this entry's targets and re-enter as `Stage::Targets`.
    NeedsTargets(Command),
    /// Run this now.
    Run(Outcome),
}

pub enum Outcome {
    Command {
        id: String,
        args: Vec<String>,
    },
    Action {
        id: String,
        plugin_id: String,
        action_id: String,
    },
}

pub struct App {
    pub stage: Stage,
    pub query: String,
    pub state: ListState,
    pub status: Option<String>,
    candidates: Vec<Candidate>,
    filtered: Vec<usize>,
    frecency: Frecency,
}

impl App {
    pub fn new(candidates: Vec<Candidate>, frecency: Frecency) -> Self {
        let mut app = App {
            stage: Stage::Commands,
            query: String::new(),
            state: ListState::default(),
            status: None,
            candidates,
            filtered: Vec::new(),
            frecency,
        };
        app.refilter();
        app
    }

    pub fn enter_targets(&mut self, command: Command, targets: Vec<Target>) {
        self.stage = Stage::Targets { command, targets };
        self.query.clear();
        self.refilter();
    }

    /// Backs out of target-picking to the command list. Returns false when
    /// already at the command list, which is where Esc means "close".
    pub fn leave_targets(&mut self) -> bool {
        if matches!(self.stage, Stage::Commands) {
            return false;
        }
        self.stage = Stage::Commands;
        self.query.clear();
        self.status = None;
        self.refilter();
        true
    }

    pub fn rows(&self) -> Vec<&str> {
        self.filtered.iter().map(|&i| self.row_title(i)).collect()
    }

    pub fn total(&self) -> usize {
        match &self.stage {
            Stage::Commands => self.candidates.len(),
            Stage::Targets { targets, .. } => targets.len(),
        }
    }

    pub fn shown(&self) -> usize {
        self.filtered.len()
    }

    fn row_title(&self, i: usize) -> &str {
        match &self.stage {
            Stage::Commands => &self.candidates[i].title,
            Stage::Targets { targets, .. } => &targets[i].label,
        }
    }

    /// A typed query filters first; frecency only orders what survives it, so a
    /// frequently-used command never outranks a better textual match (§7).
    fn refilter(&mut self) {
        let n = self.total();
        let mut scored: Vec<(i32, i64, usize)> = (0..n)
            .filter_map(|i| {
                fuzzy::score(&self.query, self.row_title(i)).map(|s| {
                    let rank = match &self.stage {
                        Stage::Commands => self.frecency.rank(&self.candidates[i].id),
                        Stage::Targets { .. } => 0.0,
                    };
                    (s, (rank * 1000.0) as i64, i)
                })
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)));
        self.filtered = scored.into_iter().map(|(_, _, i)| i).collect();
        self.state.select((!self.filtered.is_empty()).then_some(0));
    }

    pub fn move_selection(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as i32;
        let last = self.filtered.len() as i32 - 1;
        self.state
            .select(Some((cur + delta).clamp(0, last) as usize));
    }

    pub fn push(&mut self, c: char) {
        self.query.push(c);
        self.refilter();
    }

    pub fn pop(&mut self) {
        self.query.pop();
        self.refilter();
    }

    fn selected(&self) -> Option<usize> {
        self.state
            .selected()
            .and_then(|i| self.filtered.get(i))
            .copied()
    }

    pub fn confirm(&mut self) -> Step {
        let Some(i) = self.selected() else {
            return Step::Continue;
        };

        match &self.stage {
            Stage::Commands => match &self.candidates[i].kind {
                Kind::Command(c) if c.needs_target() => Step::NeedsTargets(c.clone()),
                Kind::Command(c) => {
                    let id = c.id.clone();
                    self.frecency.record(&id);
                    Step::Run(Outcome::Command {
                        id,
                        args: c.args.clone(),
                    })
                }
                Kind::Action(a) => {
                    let id = self.candidates[i].id.clone();
                    self.frecency.record(&id);
                    Step::Run(Outcome::Action {
                        id,
                        plugin_id: a.plugin_id.clone(),
                        action_id: a.action_id.clone(),
                    })
                }
            },
            Stage::Targets { command, targets } => {
                let target = &targets[i];
                let args = substitute(&command.args, &target.id);
                let id = command.id.clone();
                self.frecency.record(&id);
                Step::Run(Outcome::Command { id, args })
            }
        }
    }

    pub fn frecency(&self) -> &Frecency {
        &self.frecency
    }
}

/// Replaces the `{}` placeholder with the chosen id. Only the placeholder is
/// replaced — the surrounding argv is passed through untouched, so a target
/// whose id happens to contain a placeholder-like string cannot rewrite the
/// command around it.
fn substitute(args: &[String], id: &str) -> Vec<String> {
    args.iter()
        .map(|a| if a == "{}" { id.to_string() } else { a.clone() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Command as Cmd;

    fn cmd(id: &str, title: &str, args: &[&str], resolve: Option<&str>) -> Cmd {
        Cmd {
            id: id.into(),
            title: title.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            contexts: vec![],
            resolve: resolve.map(str::to_owned),
        }
    }

    fn app_with(cmds: Vec<Cmd>) -> App {
        App::new(
            cmds.into_iter().map(Candidate::from_command).collect(),
            Frecency::default(),
        )
    }

    #[test]
    fn substitutes_only_the_placeholder() {
        let args = ["tab".to_string(), "focus".to_string(), "{}".to_string()];
        assert_eq!(substitute(&args, "w3Y:t1"), vec!["tab", "focus", "w3Y:t1"]);
    }

    #[test]
    fn substitution_leaves_a_placeholder_shaped_id_inert() {
        let args = ["tab".to_string(), "focus".to_string(), "{}".to_string()];
        // The id lands in the placeholder slot; it does not rewrite its neighbours.
        assert_eq!(substitute(&args, "{}"), vec!["tab", "focus", "{}"]);
    }

    #[test]
    fn a_dynamic_entry_asks_for_a_target_instead_of_running() {
        let mut app = app_with(vec![cmd(
            "tab.focus",
            "Switch to tab",
            &["tab", "focus", "{}"],
            Some("tab list"),
        )]);
        assert!(matches!(app.confirm(), Step::NeedsTargets(_)));
    }

    #[test]
    fn a_fixed_entry_runs_directly() {
        let mut app = app_with(vec![cmd("tab.create", "New tab", &["tab", "create"], None)]);
        match app.confirm() {
            Step::Run(Outcome::Command { id, args }) => {
                assert_eq!(id, "tab.create");
                assert_eq!(args, vec!["tab", "create"]);
            }
            _ => panic!("expected a direct run"),
        }
    }

    #[test]
    fn picking_a_target_substitutes_it_into_the_command() {
        let command = cmd(
            "tab.focus",
            "Switch to tab",
            &["tab", "focus", "{}"],
            Some("tab list"),
        );
        let mut app = app_with(vec![command.clone()]);
        app.enter_targets(
            command,
            vec![Target {
                id: "w3Y:t1".into(),
                label: "herdr".into(),
            }],
        );
        match app.confirm() {
            Step::Run(Outcome::Command { args, .. }) => {
                assert_eq!(args, vec!["tab", "focus", "w3Y:t1"]);
            }
            _ => panic!("expected the substituted command"),
        }
    }

    #[test]
    fn query_filters_before_frecency_orders() {
        let mut frecency = Frecency::default();
        for _ in 0..50 {
            frecency.record("tab.create");
        }
        let candidates = vec![
            cmd("tab.create", "New tab", &["tab", "create"], None),
            cmd(
                "pane.split.right",
                "Split pane: right",
                &["pane", "split"],
                None,
            ),
        ];
        let mut app = App::new(
            candidates
                .into_iter()
                .map(Candidate::from_command)
                .collect(),
            frecency,
        );
        // Heavily-used "New tab" does not survive a query it does not match...
        app.query = "split".into();
        app.refilter();
        assert_eq!(app.rows(), vec!["Split pane: right"]);
        // ...but leads on an empty query, which is what frecency is for.
        app.query.clear();
        app.refilter();
        assert_eq!(app.rows()[0], "New tab");
    }

    #[test]
    fn selection_stays_inside_the_filtered_list() {
        let mut app = app_with(vec![
            cmd("a", "Alpha", &["a"], None),
            cmd("b", "Beta", &["b"], None),
        ]);
        app.move_selection(-5);
        assert_eq!(app.state.selected(), Some(0));
        app.move_selection(5);
        assert_eq!(app.state.selected(), Some(1));
    }

    #[test]
    fn esc_backs_out_of_targets_before_it_closes_the_palette() {
        let command = cmd(
            "tab.focus",
            "Switch to tab",
            &["tab", "focus", "{}"],
            Some("tab list"),
        );
        let mut app = app_with(vec![command.clone()]);
        // At the command list there is nothing to back out of.
        assert!(!app.leave_targets());

        app.enter_targets(
            command,
            vec![Target {
                id: "w3Y:t1".into(),
                label: "herdr".into(),
            }],
        );
        assert_eq!(app.rows(), vec!["herdr"]);
        assert!(app.leave_targets());
        // Back at the commands, with the target query discarded.
        assert_eq!(app.rows(), vec!["Switch to tab"]);
        assert!(app.query.is_empty());
        assert!(!app.leave_targets());
    }

    #[test]
    fn confirming_an_empty_list_does_nothing() {
        let mut app = app_with(vec![cmd("a", "Alpha", &["a"], None)]);
        app.query = "zzzz".into();
        app.refilter();
        assert!(app.rows().is_empty());
        assert!(matches!(app.confirm(), Step::Continue));
    }
}
