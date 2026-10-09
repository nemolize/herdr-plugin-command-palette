//! Candidate list and selection state, independent of how it is drawn.

use crate::catalog::Command;
use std::time::Duration;

use crate::frecency::Frecency;
use crate::generate_row::GenerateRow;
use crate::glyph::SEPARATOR;
use crate::herdr::PluginAction;
use crate::line_edit::{Edit, LineEdit};
use crate::listing::Target;
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
    /// The glyph leading the row; empty still takes its cell.
    pub icon: String,
}

/// Another plugin's action — one glyph for all of them, since the catalog that
/// would name a subject does not carry these rows.
pub const ACTION_ICON: &str = "⧉";
pub const NOTE_ICON: &str = "!";

impl Candidate {
    pub fn from_command(c: Command) -> Self {
        Self {
            id: c.id.clone(),
            title: c.title.clone(),
            icon: c.icon().to_string(),
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
            icon: ACTION_ICON.to_string(),
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
            icon: NOTE_ICON.to_string(),
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
        text: LineEdit,
        generate: Option<GenerateRow>,
    },
}

/// What the event loop should do after a keypress.
pub enum Step {
    Continue,
    Cancel,
    /// Fill this entry's `{repo}`, then hand it back through `App::picked`.
    /// Filled here, before any stage, so a typed or picked value spelling
    /// `{repo}` is never rewritten.
    NeedsRepo(Command),
    /// Fetch this entry's targets and re-enter as `Stage::Targets`.
    NeedsTargets(Command),
    /// Seed this entry's input with the current name. A `Step` rather than a
    /// direct call because the name lives behind a list API, and `App` holds no
    /// herdr handle — every herdr call is the event loop's.
    NeedsPrompt(Command),
    /// Ask the model for a name for `workspace`, as request `id`, and hand the
    /// answer back through `App::generated`.
    Generate {
        id: u64,
        workspace: String,
    },
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
    /// Whether Commands-stage rows lead with their icon (`settings.toml`).
    pub icons: bool,
    candidates: Vec<Candidate>,
    frecency: Frecency,
    /// Never reused, so an answer outliving its stage cannot match a later one.
    last_generation: u64,
}

impl App {
    pub fn new(candidates: Vec<Candidate>, frecency: Frecency) -> Self {
        let mut app = App {
            stage: Stage::Commands,
            status: None,
            selection: Selection::default(),
            icons: true,
            candidates,
            frecency,
            last_generation: 0,
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
        let generate = workspace_to_name(&args).map(|_| GenerateRow::default());
        self.stage = Stage::Prompt {
            command,
            args,
            text: LineEdit::new(seed),
            generate,
        };
        self.selection.clear_query();
        self.status = None;
    }

    /// Adds a footer note after any already showing, so a later note never
    /// hides an earlier one.
    pub fn add_status(&mut self, note: String) {
        self.status = Some(match self.status.take() {
            Some(earlier) => format!("{earlier}{SEPARATOR}{note}"),
            None => note,
        });
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

    #[cfg(test)]
    pub fn query(&self) -> &str {
        self.selection.query.text()
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

    /// Aligned with `rows`; None when there is no icon column — switched off, or
    /// a stage listing one kind of thing, where a glyph distinguishes nothing.
    pub fn row_icons(&self) -> Option<Vec<&str>> {
        if !self.icons || !matches!(self.stage, Stage::Commands) {
            return None;
        }
        Some(
            self.selection
                .visible()
                .iter()
                .map(|&i| self.candidates[i].icon.as_str())
                .collect(),
        )
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

    /// At a prompt this moves focus between the input and the generate row,
    /// which is the only other thing there; elsewhere it moves through the list.
    pub fn move_selection(&mut self, delta: i32) {
        if let Stage::Prompt { generate, .. } = &mut self.stage {
            if let Some(row) = generate.as_mut().filter(|r| !r.is_generating()) {
                row.focused = delta > 0;
            }
            return;
        }
        self.selection.move_by(delta);
    }

    pub fn generate_row(&self) -> Option<&GenerateRow> {
        match &self.stage {
            Stage::Prompt { generate, .. } => generate.as_ref(),
            _ => None,
        }
    }

    pub fn is_generating(&self) -> bool {
        self.generate_row().is_some_and(GenerateRow::is_generating)
    }

    /// Leaves the input as it was. False when nothing was running, so Esc then
    /// means what it means everywhere else.
    pub fn cancel_generation(&mut self) -> bool {
        match &mut self.stage {
            Stage::Prompt {
                generate: Some(row),
                ..
            } => row.cancel(),
            _ => false,
        }
    }

    /// A successful answer to request `id`, which took `elapsed`, replaces the
    /// typed name.
    pub fn generated(&mut self, id: u64, answer: Result<String, String>, elapsed: Duration) {
        if let Stage::Prompt {
            text,
            generate: Some(row),
            ..
        } = &mut self.stage
        {
            if let Some(name) = row.finish(id, answer, elapsed, text.text()) {
                *text = LineEdit::new(name);
                self.status = None;
            }
        }
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

    /// Applies a keystroke to whichever line is live: the typed name at the
    /// prompt, the query everywhere else.
    pub fn edit(&mut self, edit: Edit) {
        if let Stage::Prompt { text, generate, .. } = &mut self.stage {
            if generate.as_ref().is_some_and(|r| r.focused) {
                return;
            }
            // The refusal described the old text; leaving it up would report
            // `--clear` at an input that no longer says it. So did the answer.
            if text.apply(edit) {
                self.status = None;
                if let Some(row) = generate {
                    row.clear_answer();
                }
            }
            return;
        }
        // Refiltering puts the list back on its first row, which an edit that
        // left the query as it was has no reason to do.
        if self.selection.query.apply(edit) {
            self.refilter();
        }
    }

    #[cfg(test)]
    fn push_str(&mut self, s: &str) {
        for c in s.chars() {
            self.edit(Edit::Insert(c));
        }
    }

    #[cfg(test)]
    fn clear_query(&mut self) {
        self.selection.clear_query();
        self.refilter();
    }

    pub fn confirm(&mut self) -> Step {
        if let Stage::Prompt {
            generate: Some(row),
            args,
            ..
        } = &mut self.stage
        {
            if row.focused {
                let id = self.last_generation + 1;
                let Some(workspace) = workspace_to_name(args) else {
                    return Step::Continue;
                };
                if !row.start(id) {
                    return Step::Continue;
                }
                self.last_generation = id;
                return Step::Generate {
                    id,
                    workspace: workspace.to_string(),
                };
            }
        }
        if let Stage::Prompt {
            command,
            args,
            text,
            ..
        } = &self.stage
        {
            // Checked trimmed, SENT untrimmed: herdr stores surrounding spaces
            // verbatim, so trimming would rewrite a seeded name on a bare Enter.
            let text = text.text();
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
                Kind::Command(c) if c.args.iter().any(|a| a == "{repo}") => {
                    Step::NeedsRepo(c.clone())
                }
                Kind::Command(c) => {
                    let c = c.clone();
                    self.picked(c)
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

    /// What a command picked from the list needs next.
    pub fn picked(&mut self, c: Command) -> Step {
        if c.needs_target() {
            Step::NeedsTargets(c)
        } else if c.needs_text() {
            Step::NeedsPrompt(c)
        } else {
            self.frecency.record(&c.id);
            Step::Run(Outcome::Command {
                id: c.id,
                args: c.args,
            })
        }
    }

    pub fn frecency(&self) -> &Frecency {
        &self.frecency
    }
}

/// The workspace a rename entry acts on, which is the one whose panes the
/// model reads; None for every other entry. Tab and pane renames are left out
/// until their own summary is designed.
fn workspace_to_name(args: &[String]) -> Option<&str> {
    match args {
        [subject, verb, id, ..] if subject == "workspace" && verb == "rename" => {
            (!id.starts_with('{')).then_some(id.as_str())
        }
        _ => None,
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
            icon: None,
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
    fn a_later_footer_note_follows_an_earlier_one_rather_than_replacing_it() {
        let mut app = app_with(vec![]);
        app.add_status("first".into());
        assert_eq!(app.status.as_deref(), Some("first"));
        app.add_status("second".into());
        app.add_status("third".into());
        assert_eq!(app.status.as_deref(), Some("first ⋅ second ⋅ third"));
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
    fn a_repo_entry_asks_for_its_repository_before_anything_else() {
        let entry = cmd(
            "worktree.open",
            "Open worktree",
            &["worktree", "open", "--cwd", "{repo}", "--path", "{}"],
            Some("worktree list"),
        );
        let mut app = app_with(vec![entry]);
        assert!(matches!(app.confirm(), Step::NeedsRepo(_)));
        assert_eq!(app.frecency().rank("worktree.open"), 0.0);
    }

    /// Only a run is a use: a stage the user may still back out of is not.
    #[test]
    fn a_picked_entry_is_ranked_only_when_it_runs() {
        let mut app = app_with(vec![]);
        let staged = cmd(
            "tab.focus",
            "Focus tab",
            &["tab", "focus", "{}"],
            Some("tab list"),
        );
        assert!(matches!(app.picked(staged), Step::NeedsTargets(_)));
        assert_eq!(app.frecency().rank("tab.focus"), 0.0);

        let fixed = cmd("tab.create", "New tab", &["tab", "create"], None);
        assert!(matches!(app.picked(fixed), Step::Run(_)));
        assert!(app.frecency().rank("tab.create") > 0.0);
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
            "Rename tab...",
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
            app.edit(Edit::Insert(c));
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
        app.edit(Edit::Insert(' '));
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
            app.edit(Edit::Insert(c));
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
        app.edit(Edit::DeleteBack);
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
            "Rename pane...",
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
        assert_eq!(app.rows(), vec!["Rename tab..."]);
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
            "Rename pane...",
            &["pane", "rename", "w3Y:p1", "{text}"],
            None,
        );
        c.prompt = Some("New pane name".into());
        let mut app = app_with(vec![c.clone()]);
        app.enter_prompt(c, "--clear".into());
        app.confirm();
        assert!(app.status.is_some(), "refused first");

        app.edit(Edit::Left);
        assert!(app.status.is_some(), "still the text it describes");
        app.edit(Edit::End);
        app.edit(Edit::DeleteForward);
        assert!(app.status.is_some(), "a delete with nothing to delete");

        app.edit(Edit::DeleteBack);
        assert!(app.status.is_none(), "and retracted once the text changes");
    }

    /// Typing at the prompt must not reach the query, which drives the filter of
    /// the list the user is no longer looking at.
    #[test]
    fn typing_at_the_prompt_does_not_filter_the_command_list() {
        let mut app = at_prompt("");
        app.edit(Edit::Insert('z'));
        assert!(app.query().is_empty());
        app.leave_stage();
        assert_eq!(app.rows(), vec!["Rename tab..."], "list intact");
    }

    #[test]
    fn confirming_an_empty_list_does_nothing() {
        let mut app = app_with(vec![cmd("a", "Alpha", &["a"], None)]);
        app.push_str("zzzz");
        assert!(app.rows().is_empty());
        assert!(matches!(app.confirm(), Step::Continue));
    }

    fn renaming_workspace(seed: &str) -> App {
        let mut c = cmd(
            "workspace.rename",
            "Rename workspace...",
            &["workspace", "rename", "w3Y", "{text}"],
            None,
        );
        c.prompt = Some("New workspace name".into());
        let mut app = app_with(vec![c.clone()]);
        app.enter_prompt(c, seed.into());
        app
    }

    fn row(app: &App) -> &GenerateRow {
        app.generate_row().expect("a generate row")
    }

    fn typed(app: &App) -> &str {
        match &app.stage {
            Stage::Prompt { text, .. } => text.text(),
            _ => panic!("not at the prompt"),
        }
    }

    fn start_generating(app: &mut App) -> u64 {
        app.move_selection(1);
        match app.confirm() {
            Step::Generate { id, workspace } => {
                assert_eq!(workspace, "w3Y");
                id
            }
            _ => panic!("expected a generate request"),
        }
    }

    #[test]
    fn only_a_workspace_rename_offers_a_generated_name() {
        assert!(renaming_workspace("w").generate_row().is_some());
        assert!(at_prompt("t").generate_row().is_none(), "tab rename");
        let mut c = cmd(
            "pane.rename",
            "Rename pane...",
            &["pane", "rename", "w3Y:p1", "{text}"],
            None,
        );
        c.prompt = Some("New pane name".into());
        let mut app = app_with(vec![]);
        app.enter_prompt(c, String::new());
        assert!(app.generate_row().is_none(), "pane rename");
    }

    #[test]
    fn down_and_up_move_focus_between_the_input_and_the_row() {
        let mut app = renaming_workspace("herdr");
        assert!(!row(&app).focused, "the input has focus first");
        app.move_selection(1);
        assert!(row(&app).focused);
        app.move_selection(1);
        assert!(row(&app).focused, "nothing below the row");
        app.move_selection(-1);
        assert!(!row(&app).focused);
    }

    #[test]
    fn enter_on_the_row_asks_for_a_name_and_marks_it_generating() {
        let mut app = renaming_workspace("herdr");
        let id = start_generating(&mut app);
        assert!(app.is_generating());
        assert_eq!(row(&app).label(), "\u{2728} Generating...");
        assert_eq!(typed(&app), "herdr", "the input is not touched");
        assert!(
            matches!(app.confirm(), Step::Continue),
            "no second request while one runs"
        );

        app.generated(id, Err("empty answer".into()), Duration::ZERO);
        assert!(
            matches!(app.confirm(), Step::Generate { id: next, .. } if next > id),
            "a retry is a new request"
        );
    }

    #[test]
    fn enter_in_the_input_still_renames() {
        let mut app = renaming_workspace("herdr");
        match app.confirm() {
            Step::Run(Outcome::Command { args, .. }) => {
                assert_eq!(args, ["workspace", "rename", "w3Y", "herdr"]);
            }
            _ => panic!("expected the rename"),
        }
    }

    #[test]
    fn a_generated_name_replaces_the_input_and_enter_renames_to_it() {
        let mut app = renaming_workspace("herdr");
        let id = start_generating(&mut app);
        app.generated(id, Ok("palette LLM".into()), Duration::ZERO);

        assert_eq!(typed(&app), "palette LLM");
        assert!(!row(&app).focused, "focus is back on the input");
        app.edit(Edit::Insert('!'));
        assert_eq!(typed(&app), "palette LLM!", "cursor at the end");
        app.edit(Edit::DeleteBack);
        match app.confirm() {
            Step::Run(Outcome::Command { args, .. }) => {
                assert_eq!(args, ["workspace", "rename", "w3Y", "palette LLM"]);
            }
            _ => panic!("expected the rename"),
        }
    }

    #[test]
    fn the_row_says_how_long_an_answer_took_and_whether_it_changed_the_name() {
        let mut app = renaming_workspace("herdr");
        let id = start_generating(&mut app);
        app.generated(id, Ok("palette".into()), Duration::from_millis(2_340));
        assert_eq!(row(&app).label(), "\u{2728} Auto generate \u{22C5} 2.3s");

        let mut app = renaming_workspace("herdr");
        let id = start_generating(&mut app);
        app.generated(id, Ok("herdr".into()), Duration::from_millis(1_800));
        assert_eq!(
            row(&app).label(),
            "\u{2728} Auto generate \u{22C5} unchanged (1.8s)"
        );
        assert_eq!(typed(&app), "herdr");
    }

    #[test]
    fn editing_the_name_clears_what_the_row_said_about_the_answer() {
        let mut app = renaming_workspace("herdr");
        let id = start_generating(&mut app);
        app.generated(id, Ok("herdr".into()), Duration::from_millis(1_800));
        app.edit(Edit::Left);
        assert!(
            row(&app).label().contains("unchanged"),
            "a cursor move leaves the text, and the answer, as they were"
        );
        app.edit(Edit::Insert('!'));
        assert_eq!(row(&app).label(), "\u{2728} Auto generate");
    }

    #[test]
    fn cancelling_keeps_the_input_and_drops_the_late_answer() {
        let mut app = renaming_workspace("herdr");
        let id = start_generating(&mut app);
        assert!(app.cancel_generation());
        assert!(!app.is_generating());
        assert!(
            matches!(app.stage, Stage::Prompt { .. }),
            "still at the prompt"
        );

        app.generated(id, Ok("late".into()), Duration::ZERO);
        assert_eq!(typed(&app), "herdr");
        assert!(!app.cancel_generation(), "Esc now goes back as usual");
    }

    #[test]
    fn an_answer_arriving_after_the_stage_was_left_changes_nothing() {
        let mut app = renaming_workspace("herdr");
        let id = start_generating(&mut app);
        app.cancel_generation();
        app.leave_stage();
        app.generated(id, Ok("late".into()), Duration::ZERO);
        assert!(matches!(app.stage, Stage::Commands));
    }

    #[test]
    fn a_failure_leaves_the_input_and_shows_why_on_the_row() {
        for why in [
            "no Ollama at localhost:11434",
            "HTTP 404: model 'x' not found",
            "timed out after 60s",
            "empty answer",
        ] {
            let mut app = renaming_workspace("herdr");
            let id = start_generating(&mut app);
            app.generated(id, Err(why.into()), Duration::ZERO);
            assert_eq!(typed(&app), "herdr", "{why}");
            assert!(row(&app).label().ends_with(why), "{why}");
            assert!(row(&app).focused, "{why}: still selectable to retry");
        }
    }

    #[test]
    fn keys_do_not_move_focus_or_edit_while_generating() {
        let mut app = renaming_workspace("herdr");
        start_generating(&mut app);
        app.move_selection(-1);
        assert!(row(&app).focused, "focus stays while it runs");
        app.edit(Edit::Insert('x'));
        assert_eq!(typed(&app), "herdr");
    }

    #[test]
    fn typing_while_the_row_has_focus_does_not_reach_the_input() {
        let mut app = renaming_workspace("herdr");
        app.move_selection(1);
        app.edit(Edit::Insert('x'));
        assert_eq!(typed(&app), "herdr");
    }
}
