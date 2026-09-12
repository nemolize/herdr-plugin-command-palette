//! Candidate list and selection state, independent of how it is drawn.

use crate::catalog::Command;
use crate::frecency::Frecency;
use crate::herdr::{PluginAction, Target};
use crate::selection::Selection;

pub enum Kind {
    /// A catalog entry, runnable as written.
    Command(Command),
    /// Another plugin's action, invoked through `herdr plugin action invoke`.
    Action(PluginAction),
    /// A row that reports rather than runs, carrying why an entry was dropped.
    /// The list is where it goes because stderr is wiped by the alternate
    /// screen and a dropped entry is invisible in the list it is missing from.
    Note(String),
}

pub struct Candidate {
    pub id: String,
    pub title: String,
    pub kind: Kind,
    /// The key this entry is also reachable by, empty when none is known. Held
    /// beside the title rather than inside it because the two are laid out in
    /// separate columns and the title alone is what the filter matches.
    pub key: String,
}

impl Candidate {
    pub fn from_command(c: Command) -> Self {
        Self {
            id: c.id.clone(),
            title: c.title.clone(),
            kind: Kind::Command(c),
            key: String::new(),
        }
    }

    pub fn from_action(a: PluginAction) -> Self {
        Self {
            id: format!("{}.{}", a.plugin_id, a.action_id),
            title: a.title.clone(),
            kind: Kind::Action(a),
            key: String::new(),
        }
    }

    /// The reason rides in the kind rather than the title because a title is
    /// clipped to the pane width, which cuts it off mid-word.
    pub fn note(id: &str, why: &str) -> Self {
        Self {
            id: id.to_string(),
            title: format!("skipped `{id}`"),
            kind: Kind::Note(why.to_string()),
            key: String::new(),
        }
    }
}

/// What the palette is asking for right now. Picking an entry that needs an
/// argument does not run it — it asks for that argument next, so one palette
/// covers both halves of the choice.
pub enum Stage {
    Commands,
    Targets {
        command: Command,
        targets: Vec<Target>,
    },
    Prompt {
        command: Command,
        args: Vec<String>,
        text: String,
    },
}

/// What the event loop should do after a keypress.
pub enum Step {
    Continue,
    Cancel,
    /// Fetch this entry's targets and re-enter as `Stage::Targets`.
    NeedsTargets(Command),
    /// Seed this entry's input with the current name. A `Step` rather than a
    /// direct call because the name lives behind a list API, and `App` holds no
    /// herdr handle — every herdr call is the event loop's.
    NeedsPrompt(Command),
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
    pub status: Option<String>,
    pub selection: Selection,
    candidates: Vec<Candidate>,
    frecency: Frecency,
}

impl App {
    pub fn new(candidates: Vec<Candidate>, frecency: Frecency) -> Self {
        let mut app = App {
            stage: Stage::Commands,
            status: None,
            selection: Selection::default(),
            candidates,
            frecency,
        };
        app.refilter();
        app
    }

    pub fn enter_targets(&mut self, command: Command, targets: Vec<Target>) {
        self.stage = Stage::Targets { command, targets };
        self.selection.clear_query();
        self.refilter();
    }

    /// Opens the free-text stage with `seed` already typed, so renaming
    /// something is an edit of its current name rather than a retype.
    pub fn enter_prompt(&mut self, command: Command, seed: String) {
        let args = command.args.clone();
        self.stage = Stage::Prompt {
            command,
            args,
            text: seed,
        };
        self.selection.clear_query();
        self.status = None;
    }

    /// Backs out to the command list. Returns false when already there, which
    /// is where Esc means "close".
    pub fn leave_stage(&mut self) -> bool {
        if matches!(self.stage, Stage::Commands) {
            return false;
        }
        self.stage = Stage::Commands;
        self.selection.clear_query();
        self.status = None;
        self.refilter();
        true
    }

    pub fn query(&self) -> &str {
        &self.selection.query
    }

    pub fn rows(&self) -> Vec<&str> {
        self.selection
            .visible()
            .iter()
            .map(|&i| self.row_title(i))
            .collect()
    }

    /// The key beside each visible row, aligned with `rows`. Empty for a row
    /// with none, and for every row of a stage that is picking an argument
    /// rather than a command — a target has no shortcut of its own.
    pub fn row_keys(&self) -> Vec<&str> {
        self.selection
            .visible()
            .iter()
            .map(|&i| match &self.stage {
                Stage::Commands => self.candidates[i].key.as_str(),
                _ => "",
            })
            .collect()
    }

    pub fn total(&self) -> usize {
        match &self.stage {
            Stage::Commands => self.candidates.len(),
            Stage::Targets { targets, .. } => targets.len(),
            Stage::Prompt { .. } => 0,
        }
    }

    pub fn shown(&self) -> usize {
        self.selection.shown()
    }

    fn row_title(&self, i: usize) -> &str {
        match &self.stage {
            Stage::Commands => &self.candidates[i].title,
            Stage::Targets { targets, .. } => &targets[i].label,
            Stage::Prompt { .. } => "",
        }
    }

    fn refilter(&mut self) {
        let rows: Vec<String> = (0..self.total())
            .map(|i| self.row_title(i).to_string())
            .collect();
        let rank: Vec<f64> = match self.stage {
            Stage::Commands => self
                .candidates
                .iter()
                .map(|c| self.frecency.rank(&c.id))
                .collect(),
            _ => vec![0.0; rows.len()],
        };
        let borrowed: Vec<&str> = rows.iter().map(String::as_str).collect();
        self.selection.refilter(&borrowed, |i| rank[i]);
    }

    pub fn move_selection(&mut self, delta: i32) {
        self.selection.move_by(delta);
    }

    pub fn selected_note(&self) -> Option<&str> {
        if !matches!(self.stage, Stage::Commands) {
            return None;
        }
        match &self.candidates.get(self.selection.selected()?)?.kind {
            Kind::Note(why) => Some(why),
            _ => None,
        }
    }

    pub fn push(&mut self, c: char) {
        self.selection.query.push(c);
        self.refilter();
    }

    pub fn pop(&mut self) {
        self.selection.query.pop();
        self.refilter();
    }

    #[cfg(test)]
    fn push_str(&mut self, s: &str) {
        for c in s.chars() {
            self.push(c);
        }
    }

    #[cfg(test)]
    fn clear_query(&mut self) {
        self.selection.clear_query();
        self.refilter();
    }

    pub fn confirm(&mut self) -> Step {
        if let Stage::Prompt {
            command,
            args,
            text,
        } = &self.stage
        {
            // Checked trimmed, SENT untrimmed: herdr stores surrounding spaces
            // verbatim, so trimming would rewrite a seeded name on a bare Enter.
            if text.trim().is_empty() {
                return Step::Continue;
            }
            if eaten_as_a_flag(args, text) {
                self.status = Some(format!("`{text}` is a flag here, not a name"));
                return Step::Continue;
            }
            let id = command.id.clone();
            let args = substitute(args, "{text}", text);
            self.frecency.record(&id);
            return Step::Run(Outcome::Command { id, args });
        }

        let Some(i) = self.selection.selected() else {
            return Step::Continue;
        };

        match &self.stage {
            Stage::Commands => match &self.candidates[i].kind {
                Kind::Command(c) if c.needs_target() => Step::NeedsTargets(c.clone()),
                Kind::Command(c) if c.needs_text() => Step::NeedsPrompt(c.clone()),
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
                Kind::Note(_) => Step::Continue,
            },
            Stage::Targets { command, targets } => {
                let target = &targets[i];
                let args = substitute(&command.args, "{}", &target.id);
                let id = command.id.clone();
                self.frecency.record(&id);
                Step::Run(Outcome::Command { id, args })
            }
            Stage::Prompt { .. } => Step::Continue,
        }
    }

    /// Edits the free-text stage's buffer. Separate from `push` / `pop`, which
    /// drive the query and refilter a list this stage does not have.
    pub fn push_text(&mut self, c: char) {
        if let Stage::Prompt { text, .. } = &mut self.stage {
            text.push(c);
            // The refusal described the old text; leaving it up would report
            // `--clear` at an input that no longer says it.
            self.status = None;
        }
    }

    pub fn pop_text(&mut self) {
        if let Stage::Prompt { text, .. } = &mut self.stage {
            text.pop();
            self.status = None;
        }
    }

    pub fn frecency(&self) -> &Frecency {
        &self.frecency
    }
}

/// Whether `text` would reach the command as one of its own flags.
///
/// Measured on 0.9.0: `pane rename --clear` deletes the name and exits 0, while
/// every other `-`-leading name is stored verbatim by all three commands.
///
/// Matched on the subcommand pair rather than the first argument, so a leading
/// global flag (`--session x pane rename …`) cannot walk past the guard and a
/// sibling subcommand (`pane send-text`) is not caught by it.
fn eaten_as_a_flag(args: &[String], text: &str) -> bool {
    if text != "--clear" {
        return false;
    }
    // `--session` and `--remote` consume the token after them, which would
    // otherwise read as the subcommand and walk the pair out of alignment.
    const GLOBAL_FLAGS_TAKING_A_VALUE: [&str; 3] =
        ["--session", "--remote", "--remote-keybindings"];

    let mut rest = args.iter().map(String::as_str);
    let mut verbs = Vec::new();
    while let Some(a) = rest.next() {
        if GLOBAL_FLAGS_TAKING_A_VALUE.contains(&a) {
            rest.next();
        } else if !a.starts_with('-') {
            verbs.push(a);
            if verbs.len() == 2 {
                break;
            }
        }
    }
    matches!(verbs.as_slice(), ["pane", "rename"])
}

/// Fills one placeholder — `{}` with a chosen id, `{text}` with a typed name.
///
/// The value lands in exactly one argv element and the surrounding argv is
/// untouched, so a value that looks like a placeholder cannot rewrite the
/// command around it, and a name with spaces cannot split into several
/// arguments to a variadic `LABEL...`.
fn substitute(args: &[String], placeholder: &str, value: &str) -> Vec<String> {
    args.iter()
        .map(|a| {
            if a == placeholder {
                value.to_string()
            } else {
                a.clone()
            }
        })
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
            prompt: None,
            binding: None,
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
        assert_eq!(
            substitute(&args, "{}", "w3Y:t1"),
            vec!["tab", "focus", "w3Y:t1"]
        );
    }

    #[test]
    fn substitution_leaves_a_placeholder_shaped_id_inert() {
        let args = ["tab".to_string(), "focus".to_string(), "{}".to_string()];
        // The id lands in the placeholder slot; it does not rewrite its neighbours.
        assert_eq!(substitute(&args, "{}", "{}"), vec!["tab", "focus", "{}"]);
    }

    #[test]
    fn a_dynamic_entry_asks_for_a_target_instead_of_running() {
        let mut app = app_with(vec![cmd(
            "tab.focus",
            "Focus tab",
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
            "Focus tab",
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
        app.push_str("split");
        assert_eq!(app.rows(), vec!["Split pane: right"]);
        // ...but leads on an empty query, which is what frecency is for.
        app.clear_query();
        assert_eq!(app.rows()[0], "New tab");
    }

    #[test]
    fn esc_backs_out_of_targets_before_it_closes_the_palette() {
        let command = cmd(
            "tab.focus",
            "Focus tab",
            &["tab", "focus", "{}"],
            Some("tab list"),
        );
        let mut app = app_with(vec![command.clone()]);
        // At the command list there is nothing to back out of.
        assert!(!app.leave_stage());

        app.enter_targets(
            command,
            vec![Target {
                id: "w3Y:t1".into(),
                label: "herdr".into(),
            }],
        );
        assert_eq!(app.rows(), vec!["herdr"]);
        assert!(app.leave_stage());
        // Back at the commands, with the target query discarded.
        assert_eq!(app.rows(), vec!["Focus tab"]);
        assert!(app.query().is_empty());
        assert!(!app.leave_stage());
    }

    /// A rename entry as the palette sees it: the context id is already
    /// substituted by the time an entry becomes a candidate (`main.rs`).
    fn renamer() -> Cmd {
        let mut c = cmd(
            "tab.rename",
            "Rename tab…",
            &["tab", "rename", "w3Y:t1", "{text}"],
            None,
        );
        c.prompt = Some("New tab name".into());
        c
    }

    /// Drives a renamer to its typing stage, seeded as the loop would seed it.
    fn at_prompt(seed: &str) -> App {
        let mut app = app_with(vec![renamer()]);
        let Step::NeedsPrompt(command) = app.confirm() else {
            panic!("expected the entry to ask for text");
        };
        app.enter_prompt(command, seed.to_string());
        app
    }

    #[test]
    fn a_prompt_entry_asks_for_text_instead_of_running() {
        let mut app = app_with(vec![renamer()]);
        match app.confirm() {
            Step::NeedsPrompt(c) => assert_eq!(c.id, "tab.rename"),
            _ => panic!("expected the entry to ask for text"),
        }
    }

    #[test]
    fn typing_a_name_and_confirming_substitutes_it() {
        let mut app = at_prompt("");
        for c in "docs".chars() {
            app.push_text(c);
        }
        match app.confirm() {
            Step::Run(Outcome::Command { id, args }) => {
                assert_eq!(id, "tab.rename");
                assert_eq!(args, vec!["tab", "rename", "w3Y:t1", "docs"]);
            }
            _ => panic!("expected the typed rename"),
        }
    }

    /// The CLI's LABEL is a required positional, so submitting nothing would
    /// fail on dispatch. The stage holds instead, where the user can still type.
    #[test]
    fn an_empty_name_is_not_submitted() {
        let mut app = at_prompt("");
        assert!(matches!(app.confirm(), Step::Continue));
        app.push_text(' ');
        assert!(
            matches!(app.confirm(), Step::Continue),
            "whitespace is empty"
        );
    }

    /// A name with spaces stays ONE argv element: `<LABEL>...` is variadic, so
    /// splitting it here would pass the second word as its own argument.
    #[test]
    fn a_multi_word_name_stays_a_single_argument() {
        let mut app = at_prompt("");
        for c in "release notes".chars() {
            app.push_text(c);
        }
        match app.confirm() {
            Step::Run(Outcome::Command { args, .. }) => {
                assert_eq!(args, vec!["tab", "rename", "w3Y:t1", "release notes"]);
            }
            _ => panic!("expected the typed rename"),
        }
    }

    #[test]
    fn the_input_starts_from_the_current_name() {
        let mut app = at_prompt("herdr");
        app.pop_text();
        match app.confirm() {
            Step::Run(Outcome::Command { args, .. }) => {
                assert_eq!(args.last().unwrap(), "herd", "seeded, then edited");
            }
            _ => panic!("expected the typed rename"),
        }
    }

    /// herdr stores a name's surrounding spaces verbatim (measured on 0.9.0), so
    /// a seeded `"  a  "` must submit as itself — trimming would rename it on a
    /// bare Enter, which is a silent edit the user never asked for.
    #[test]
    fn surrounding_spaces_are_sent_as_typed() {
        let mut app = at_prompt("  review name  ");
        match app.confirm() {
            Step::Run(Outcome::Command { args, .. }) => {
                assert_eq!(args.last().unwrap(), "  review name  ");
            }
            _ => panic!("expected the typed rename"),
        }
    }

    /// `pane rename --clear` DELETES the name and reports success, so submitting
    /// `--clear` as a name would read as a rename that silently did nothing.
    #[test]
    fn a_name_the_command_would_read_as_a_flag_is_refused() {
        let mut c = cmd(
            "pane.rename",
            "Rename pane…",
            &["pane", "rename", "w3Y:p1", "{text}"],
            None,
        );
        c.prompt = Some("New pane name".into());
        let mut app = app_with(vec![c.clone()]);
        app.enter_prompt(c, "--clear".into());
        assert!(matches!(app.confirm(), Step::Continue));
        assert!(app.status.is_some(), "the refusal is said, not silent");
    }

    /// The guard is scoped to the one command that has the flag: `tab rename`
    /// stores `--clear` as an ordinary name (measured on 0.9.0).
    #[test]
    fn the_flag_guard_does_not_reach_a_command_without_that_flag() {
        let mut app = at_prompt("--clear");
        match app.confirm() {
            Step::Run(Outcome::Command { args, .. }) => {
                assert_eq!(args.last().unwrap(), "--clear");
            }
            _ => panic!("a tab may be named --clear"),
        }
    }

    /// The guard reads the subcommand pair, so neither a global flag in front of
    /// it nor a sibling `pane` subcommand shifts what it matches.
    #[test]
    fn the_flag_guard_matches_the_subcommand_not_the_first_argument() {
        let argv =
            |parts: &[&str]| -> Vec<String> { parts.iter().map(|s| s.to_string()).collect() };

        assert!(eaten_as_a_flag(&argv(&["pane", "rename", "p1"]), "--clear"));
        assert!(
            eaten_as_a_flag(
                &argv(&["--session", "work", "pane", "rename", "p1"]),
                "--clear"
            ),
            "a value-taking global flag must not shift the pair"
        );
        assert!(
            !eaten_as_a_flag(&argv(&["pane", "send-text", "p1"]), "--clear"),
            "a sibling subcommand has no such flag"
        );
        assert!(!eaten_as_a_flag(&argv(&["tab", "rename", "t1"]), "--clear"));
        assert!(
            !eaten_as_a_flag(&argv(&["pane", "rename", "p1"]), "-x"),
            "only that one literal is eaten"
        );
    }

    #[test]
    fn a_seeded_name_submits_without_being_retyped() {
        let mut app = at_prompt("herdr");
        match app.confirm() {
            Step::Run(Outcome::Command { args, .. }) => {
                assert_eq!(args.last().unwrap(), "herdr");
            }
            _ => panic!("expected the typed rename"),
        }
    }

    /// Esc backs out of the typed name to the command list rather than closing,
    /// so a rename opened by mistake is not a one-way door out of the palette.
    #[test]
    fn esc_leaves_the_prompt_without_closing_the_palette() {
        let mut app = at_prompt("herdr");
        assert!(app.leave_stage());
        assert_eq!(app.rows(), vec!["Rename tab…"]);
        assert!(!app.leave_stage());
    }

    /// A dropped entry is invisible in the list it is missing from, so it comes
    /// back as a row that says why — reachable by the query the footer names,
    /// and inert when picked, since there is nothing to run.
    #[test]
    fn a_skipped_entry_reports_itself_as_an_inert_row() {
        let mut app = App::new(
            vec![Candidate::note("tab.rename", "`prompt` without a `{text}`")],
            Frecency::default(),
        );
        app.push_str("skipped");
        assert_eq!(app.rows(), vec!["skipped `tab.rename`"]);
        assert!(matches!(app.confirm(), Step::Continue), "runs nothing");
        assert_eq!(
            app.selected_note(),
            Some("`prompt` without a `{text}`"),
            "the reason rides with the row that reports it"
        );
    }

    /// A refusal describes the text that was submitted, so editing the text has
    /// to retract it — otherwise `--clea` still reports `--clear`.
    #[test]
    fn editing_the_name_clears_a_stale_refusal() {
        let mut c = cmd(
            "pane.rename",
            "Rename pane…",
            &["pane", "rename", "w3Y:p1", "{text}"],
            None,
        );
        c.prompt = Some("New pane name".into());
        let mut app = app_with(vec![c.clone()]);
        app.enter_prompt(c, "--clear".into());
        app.confirm();
        assert!(app.status.is_some(), "refused first");

        app.pop_text();
        assert!(app.status.is_none(), "and retracted on the next keystroke");
    }

    /// Typing at the prompt must not reach the query, which drives the filter of
    /// the list the user is no longer looking at.
    #[test]
    fn typing_at_the_prompt_does_not_filter_the_command_list() {
        let mut app = at_prompt("");
        app.push_text('z');
        assert!(app.query().is_empty());
        app.leave_stage();
        assert_eq!(app.rows(), vec!["Rename tab…"], "list intact");
    }

    #[test]
    fn confirming_an_empty_list_does_nothing() {
        let mut app = app_with(vec![cmd("a", "Alpha", &["a"], None)]);
        app.push_str("zzzz");
        assert!(app.rows().is_empty());
        assert!(matches!(app.confirm(), Step::Continue));
    }
}
