//! The event loop: reading a key and turning it into a step for the pane's
//! driver. The pane entrypoint has a real TTY, unlike the action hop
//! (docs/design.md §3).
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::app::{App, Step};
use crate::line_edit::Edit;
use crate::ui::MIN_ROWS;

/// Dismissal is Esc or picking an entry — there is no click-outside-to-dismiss,
/// because no mouse events reach a plugin at all, and the palette's own binding
/// cannot close it on 0.8.2 (§6).
///
/// `drawn_rows` is the height of the frame on screen, not the terminal's size
/// now: a key typed at a "too short" frame must not act on a list the popup
/// grew back into before the key was read.
pub fn next_step(app: &mut App, drawn_rows: u16) -> Result<Step, String> {
    loop {
        // While a name is being generated the loop must come back without a
        // keypress, to pick up the answer the moment it lands.
        if app.is_generating() && !event::poll(ANSWER_POLL).map_err(|e| e.to_string())? {
            return Ok(Step::Continue);
        }
        let event = event::read().map_err(|e| e.to_string())?;
        if let Some(step) = apply(app, event, drawn_rows) {
            return Ok(step);
        }
    }
}

const ANSWER_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// One event against the state. `None` means the event carried nothing to act
/// on and the loop should read again — separated from `next_step` so a key
/// sequence can be driven through the same path a keypress takes, without a
/// terminal (`wiring_tests`).
fn apply(app: &mut App, event: Event, drawn_rows: u16) -> Option<Step> {
    // A resize has to redraw immediately rather than wait for a keypress:
    // on Termux the popup resizes exactly when the software keyboard is
    // raised, which is the moment the palette is being used (§5).
    if matches!(event, Event::Resize(_, _)) {
        return Some(Step::Continue);
    }
    let Event::Key(key) = event else {
        return None;
    };
    if key.kind != KeyEventKind::Press {
        return None;
    }
    // Ctrl-C is the other reflex for "get me out of here", and a palette
    // that ignored it would read as hung.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Step::Cancel);
    }
    // Nothing the user cannot see may change or run.
    if drawn_rows < MIN_ROWS {
        return Some(if key.code == KeyCode::Esc {
            Step::Cancel
        } else {
            Step::Continue
        });
    }
    let ctrl = key.modifiers == KeyModifiers::CONTROL;
    Some(match key.code {
        KeyCode::Esc => {
            if app.cancel_generation() || app.leave_stage() {
                Step::Continue
            } else {
                Step::Cancel
            }
        }
        KeyCode::Up => {
            app.move_selection(-1);
            Step::Continue
        }
        KeyCode::Char('p') if ctrl => {
            app.move_selection(-1);
            Step::Continue
        }
        KeyCode::Down => {
            app.move_selection(1);
            Step::Continue
        }
        KeyCode::Char('n') if ctrl => {
            app.move_selection(1);
            Step::Continue
        }
        KeyCode::Enter => app.confirm(),
        _ => {
            if let Some(edit) = Edit::from_key(key) {
                app.edit(edit);
            }
            Step::Continue
        }
    })
}

/// Keys in, argv out: the join between a keypress and `App::confirm`, which
/// `app::tests` (state built by hand) and `ui::render_tests` (what is drawn) each stop short of.
#[cfg(test)]
mod wiring_tests {
    use super::*;
    use crate::app::Stage;
    use crate::app::{Candidate, Outcome};
    use crate::catalog::Command;
    use crate::frecency::Frecency;
    use crate::herdr::PluginAction;
    use crate::listing::Target;
    use crate::ui::render;
    use crossterm::event::{KeyEvent, KeyEventState};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn command(id: &str, title: &str, args: &[&str], resolve: Option<&str>) -> Command {
        Command {
            id: id.to_string(),
            title: title.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            contexts: Vec::new(),
            resolve: resolve.map(str::to_string),
            prompt: None,
            binding: None,
            retired_icon: None,
        }
    }

    fn app_with(commands: Vec<Command>) -> App {
        App::new(
            commands.into_iter().map(Candidate::from_command).collect(),
            Frecency::default(),
        )
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    fn ctrl(c: char) -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
    }

    fn press(app: &mut App, typed: &str, keys: &[KeyCode]) -> Option<Step> {
        press_at(MIN_ROWS, app, typed, keys)
    }

    fn press_at(rows: u16, app: &mut App, typed: &str, keys: &[KeyCode]) -> Option<Step> {
        let codes = typed.chars().map(KeyCode::Char).chain(keys.iter().copied());
        for code in codes {
            match apply(app, key(code), rows) {
                None | Some(Step::Continue) => {}
                Some(step) => return Some(step),
            }
        }
        None
    }

    fn renaming_workspace() -> App {
        let mut picked = command(
            "workspace.rename",
            "Rename workspace...",
            &["workspace", "rename", "w3Y", "{text}"],
            None,
        );
        picked.prompt = Some("New workspace name".into());
        let mut app = app_with(vec![picked.clone()]);
        app.enter_prompt(picked, "herdr".into());
        app
    }

    #[test]
    fn down_then_enter_asks_for_a_name_and_esc_cancels_it_in_place() {
        let mut app = renaming_workspace();
        let step = press(&mut app, "", &[KeyCode::Down, KeyCode::Enter]);
        let Some(Step::Generate { id, .. }) = step else {
            panic!("{}", describe(&step));
        };

        let step = apply(&mut app, key(KeyCode::Esc), MIN_ROWS);
        assert!(matches!(step, Some(Step::Continue)), "{}", describe(&step));
        assert!(!app.is_generating());
        assert!(
            matches!(app.stage, Stage::Prompt { .. }),
            "the palette stays open at the prompt"
        );
        app.generated(id, Ok("late".into()), std::time::Duration::ZERO);

        let step = press(&mut app, "", &[KeyCode::Up, KeyCode::Enter]);
        match step {
            Some(Step::Run(Outcome::Command { args, .. })) => {
                assert_eq!(args, ["workspace", "rename", "w3Y", "herdr"]);
            }
            _ => panic!("{}", describe(&step)),
        }
    }

    #[test]
    fn esc_with_nothing_generating_still_goes_back() {
        let mut app = renaming_workspace();
        press(&mut app, "", &[KeyCode::Down]);
        let step = apply(&mut app, key(KeyCode::Esc), MIN_ROWS);
        assert!(matches!(step, Some(Step::Continue)), "{}", describe(&step));
        assert!(matches!(app.stage, Stage::Commands));
    }

    #[test]
    fn typing_and_pressing_enter_produces_the_argv_herdr_is_run_with() {
        let mut app = app_with(vec![
            command("tab.create", "New tab", &["tab", "create"], None),
            command(
                "pane.split.right",
                "Split pane: right",
                &["pane", "split", "--direction", "right"],
                None,
            ),
        ]);

        match press(&mut app, "split", &[KeyCode::Enter]) {
            Some(Step::Run(Outcome::Command { id, args })) => {
                assert_eq!(id, "pane.split.right");
                assert_eq!(args, ["pane", "split", "--direction", "right"]);
            }
            other => panic!("nothing ran: {}", describe(&other)),
        }
    }

    /// A `resolve` entry must not run on the first Enter — it asks for targets
    /// — and the second Enter is what carries the picked id into argv.
    #[test]
    fn a_target_entry_runs_only_after_its_target_is_picked() {
        let entry = command(
            "tab.focus",
            "Focus tab",
            &["tab", "focus", "{}"],
            Some("tab list"),
        );
        let mut app = app_with(vec![entry.clone()]);

        match press(&mut app, "focus", &[KeyCode::Enter]) {
            Some(Step::NeedsTargets(c)) => assert_eq!(c.id, "tab.focus"),
            other => panic!("expected a target prompt: {}", describe(&other)),
        }

        app.enter_targets(
            entry,
            vec![
                Target {
                    id: "w46:t1".into(),
                    label: "one".into(),
                },
                Target {
                    id: "w3Y:t1".into(),
                    label: "two".into(),
                },
            ],
        );

        match press(&mut app, "", &[KeyCode::Down, KeyCode::Enter]) {
            Some(Step::Run(Outcome::Command { args, .. })) => {
                assert_eq!(args, ["tab", "focus", "w3Y:t1"]);
            }
            other => panic!("the target did not run: {}", describe(&other)),
        }
    }

    /// Stops at the Outcome because the argv is assembled in `main`, out of
    /// this module's reach; `argv`'s own test there covers that half.
    #[test]
    fn a_plugin_action_leaves_with_the_ids_its_invoke_needs() {
        let action = PluginAction {
            plugin_id: "notes".into(),
            action_id: "capture".into(),
            title: "Capture a note".into(),
            contexts: Vec::new(),
            platforms: Vec::new(),
        };
        let mut app = App::new(vec![Candidate::from_action(action)], Frecency::default());

        match press(&mut app, "capture", &[KeyCode::Enter]) {
            Some(Step::Run(Outcome::Action {
                id,
                plugin_id,
                action_id,
            })) => {
                assert_eq!(id, "notes.capture");
                assert_eq!(plugin_id, "notes");
                assert_eq!(action_id, "capture");
            }
            other => panic!("the action did not run: {}", describe(&other)),
        }
    }

    /// Enter on a query matching nothing must not run whatever was selected
    /// before the query narrowed the list to zero.
    #[test]
    fn enter_on_an_empty_list_runs_nothing() {
        let mut app = app_with(vec![command(
            "tab.create",
            "New tab",
            &["tab", "create"],
            None,
        )]);
        assert!(press(&mut app, "zzzz", &[KeyCode::Enter]).is_none());
    }

    /// A query that cannot be corrected leaves the palette taking Enter with
    /// nothing to run, which reads as the keypress being ignored.
    #[test]
    fn backspace_restores_a_candidate_a_typo_filtered_out() {
        let mut app = app_with(vec![command(
            "tab.create",
            "New tab",
            &["tab", "create"],
            None,
        )]);
        assert!(press(&mut app, "tabz", &[KeyCode::Enter]).is_none());

        match press(&mut app, "", &[KeyCode::Backspace, KeyCode::Enter]) {
            Some(Step::Run(Outcome::Command { id, .. })) => assert_eq!(id, "tab.create"),
            other => panic!("backspace did not restore it: {}", describe(&other)),
        }
    }

    #[test]
    fn ctrl_u_clears_the_query_and_brings_every_row_back() {
        let mut app = app_with(vec![
            command("split.right", "Split pane: right", &["split"], None),
            command("tab.create", "New tab", &["tab"], None),
        ]);
        assert!(press(&mut app, "split", &[]).is_none());
        assert_eq!(app.shown(), 1);

        assert!(apply(&mut app, ctrl('u'), MIN_ROWS).is_some());
        assert_eq!(app.query(), "");
        assert_eq!(app.shown(), 2);
    }

    /// The seeded name is edited in place: the cursor goes to its start and
    /// what is typed there lands in front of it.
    #[test]
    fn a_name_is_edited_where_the_cursor_stands() {
        let mut picked = command(
            "tab.rename",
            "Rename tab...",
            &["tab", "rename", "t1", "{text}"],
            None,
        );
        picked.prompt = Some("N".into());
        let mut app = app_with(vec![picked.clone()]);
        app.enter_prompt(picked, "editor".into());

        assert!(apply(&mut app, ctrl('a'), MIN_ROWS).is_some());
        match press(&mut app, "my ", &[KeyCode::Enter]) {
            Some(Step::Run(Outcome::Command { args, .. })) => {
                assert_eq!(args, ["tab", "rename", "t1", "my editor"]);
            }
            other => panic!("expected the edited name to run: {}", describe(&other)),
        }
    }

    #[test]
    fn ctrl_n_and_ctrl_p_move_the_selection_like_down_and_up() {
        let mut app = app_with(vec![
            command("split.right", "Split pane: right", &["split"], None),
            command("split.down", "Split pane: down", &["split"], None),
        ]);
        apply(&mut app, ctrl('n'), MIN_ROWS);
        assert_eq!(app.selection.state.selected(), Some(1));
        apply(&mut app, ctrl('p'), MIN_ROWS);
        assert_eq!(app.selection.state.selected(), Some(0));
    }

    /// Only an edit that changes the query refilters, which is what puts the
    /// list back on its first row.
    #[test]
    fn moving_the_cursor_keeps_the_selected_row() {
        let mut app = app_with(vec![
            command("split.right", "Split pane: right", &["split"], None),
            command("split.down", "Split pane: down", &["split"], None),
        ]);
        assert!(press(
            &mut app,
            "split",
            &[KeyCode::Down, KeyCode::Left, KeyCode::Home]
        )
        .is_none());
        assert_eq!(app.selection.state.selected(), Some(1));
    }

    #[test]
    fn a_delete_with_nothing_to_delete_keeps_the_selected_row() {
        let mut app = app_with(vec![
            command("split.right", "Split pane: right", &["split"], None),
            command("split.down", "Split pane: down", &["split"], None),
        ]);
        assert!(press(&mut app, "split", &[KeyCode::Down, KeyCode::Delete]).is_none());
        assert_eq!(app.selection.state.selected(), Some(1));
    }

    #[test]
    fn a_too_short_palette_changes_nothing_and_runs_nothing() {
        // Three matches with the middle one selected, so Up and Down would
        // each move the selection rather than clamp.
        let mut app = app_with(vec![
            command("split.right", "Split pane: right", &["split"], None),
            command("split.down", "Split pane: down", &["split"], None),
            command("split.left", "Split pane: left", &["split"], None),
        ]);
        assert!(press(&mut app, "split", &[KeyCode::Down]).is_none());
        let selected = app.selection.selected();

        let short = MIN_ROWS - 1;
        for code in [
            KeyCode::Char('x'),
            KeyCode::Backspace,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Enter,
        ] {
            let step = press_at(short, &mut app, "", &[code]);
            assert!(step.is_none(), "{code:?}: {}", describe(&step));
            assert_eq!(app.query(), "split", "{code:?}");
            assert_eq!(app.selection.selected(), selected, "{code:?}");
            assert!(matches!(app.stage, Stage::Commands), "{code:?}");
        }
        assert!(matches!(
            apply(&mut app, ctrl('u'), short),
            Some(Step::Continue)
        ));
        assert_eq!(app.query(), "split", "Ctrl-U");

        let mut terminal = Terminal::new(TestBackend::new(36, MIN_ROWS)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let input: String = (0..36)
            .map(|x| terminal.backend().buffer()[(x, 0)].symbol())
            .collect();
        assert_eq!(input.trim_end(), "> split");
    }

    /// Esc closes outright rather than stepping back a stage: the stage it
    /// would return to is just as invisible.
    #[test]
    fn esc_and_ctrl_c_close_a_too_short_palette_from_the_targets_stage() {
        let entry = command("tab.focus", "Focus tab", &["tab", "focus", "{}"], None);
        let mut app = app_with(vec![entry.clone()]);
        app.enter_targets(
            entry,
            vec![Target {
                id: "w46:t1".into(),
                label: "one".into(),
            }],
        );

        let short = MIN_ROWS - 1;
        let step = press_at(short, &mut app, "", &[KeyCode::Esc]);
        assert!(matches!(step, Some(Step::Cancel)), "{}", describe(&step));

        let ctrl_c = Event::Key(KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        let step = apply(&mut app, ctrl_c, short);
        assert!(matches!(step, Some(Step::Cancel)), "{}", describe(&step));
    }

    /// Every failure above is "the palette accepted the key and ran nothing",
    /// so the panic has to say what it did instead.
    fn describe(step: &Option<Step>) -> String {
        match step {
            None => "the keys were consumed and no step was produced".into(),
            Some(Step::Continue) => "Continue".into(),
            Some(Step::Cancel) => "Cancel".into(),
            Some(Step::NeedsRepo(c)) => format!("NeedsRepo({})", c.id),
            Some(Step::NeedsTargets(c)) => format!("NeedsTargets({})", c.id),
            Some(Step::NeedsPrompt(c)) => format!("NeedsPrompt({})", c.id),
            Some(Step::Generate { id, subject }) => format!("Generate({id}, {subject:?})"),
            Some(Step::Run(Outcome::Command { id, args })) => {
                format!("Run({id}: {})", args.join(" "))
            }
            Some(Step::Run(Outcome::Action { id, .. })) => format!("Run(action {id})"),
        }
    }
}
