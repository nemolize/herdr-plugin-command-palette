//! Rendering and the event loop. The pane entrypoint has a real TTY, unlike the
//! action hop (docs/design.md §3).
use std::io::{stdout, Stdout};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

use crate::app::{App, Stage, Step};

/// Restores the terminal on drop, so an error path cannot leave the pane in raw
/// mode with the alternate screen still up.
pub struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Screen {
    /// Each failure undoes what it got through, because `Drop` restores the
    /// terminal only once a `Screen` exists — a bare `?` would leave it raw.
    pub fn enter() -> Result<Self, String> {
        enable_raw_mode().map_err(|e| e.to_string())?;

        if let Err(e) = stdout().execute(EnterAlternateScreen) {
            let _ = disable_raw_mode();
            return Err(e.to_string());
        }

        match Terminal::new(CrosstermBackend::new(stdout())) {
            Ok(terminal) => Ok(Screen { terminal }),
            Err(e) => {
                let _ = stdout().execute(LeaveAlternateScreen);
                let _ = disable_raw_mode();
                Err(e.to_string())
            }
        }
    }

    pub fn draw(&mut self, app: &mut App) -> Result<(), String> {
        self.terminal
            .draw(|f| render(f, app))
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = stdout().execute(LeaveAlternateScreen);
    }
}

/// Dismissal is Esc or picking an entry — there is no click-outside-to-dismiss,
/// because no mouse events reach a plugin at all, and the palette's own binding
/// cannot close it on 0.8.2 (§6).
pub fn next_step(app: &mut App) -> Result<Step, String> {
    loop {
        let event = event::read().map_err(|e| e.to_string())?;
        if let Some(step) = apply(app, event) {
            return Ok(step);
        }
    }
}

/// One event against the state. `None` means the event carried nothing to act
/// on and the loop should read again — separated from `next_step` so a key
/// sequence can be driven through the same path a keypress takes, without a
/// terminal (`wiring_tests`).
fn apply(app: &mut App, event: Event) -> Option<Step> {
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
    Some(match key.code {
        // From the target list Esc backs out to the commands rather than
        // closing: picking a dynamic entry would otherwise be a one-way
        // door out of the palette.
        KeyCode::Esc => {
            if app.leave_targets() {
                Step::Continue
            } else {
                Step::Cancel
            }
        }
        KeyCode::Up => {
            app.move_selection(-1);
            Step::Continue
        }
        KeyCode::Down => {
            app.move_selection(1);
            Step::Continue
        }
        KeyCode::Enter => app.confirm(),
        KeyCode::Backspace => {
            app.pop();
            Step::Continue
        }
        // Modifiers are excluded so a chord (Ctrl-A, Alt-f) is not typed
        // into the query as its bare letter. SHIFT is what produces capitals
        // and belongs in the text.
        KeyCode::Char(c) if (key.modifiers - KeyModifiers::SHIFT).is_empty() => {
            app.push(c);
            Step::Continue
        }
        _ => Step::Continue,
    })
}

fn render(f: &mut Frame, app: &mut App) {
    // Herdr draws the pane's own frame; a second Block here doubled it. Its
    // title is static, so Targets shows the command name on a line of its own.
    let header = match &app.stage {
        Stage::Commands => None,
        Stage::Targets { command, .. } => Some(command.title.clone()),
    };

    // Built before the layout because its wrapped height is what the status
    // row has to be given.
    let status = app
        .status
        .as_ref()
        .map(|msg| Paragraph::new(msg.clone()).wrap(Wrap { trim: false }).dim());
    let status_height = match &status {
        Some(p) => wrapped_height(p, f.area()),
        None => 1,
    };

    let chunks = Layout::vertical([
        Constraint::Length(if header.is_some() { 1 } else { 0 }),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(status_height),
    ])
    .split(f.area());

    if let Some(title) = header {
        f.render_widget(Paragraph::new(title).bold(), chunks[0]);
    }

    f.render_widget(Paragraph::new(format!("> {}", app.query)), chunks[1]);

    // Owned rather than borrowed: the list borrows `app` immutably while
    // render_stateful_widget needs `app.state` mutably.
    let rows: Vec<String> = app.rows().into_iter().map(str::to_owned).collect();
    if rows.is_empty() {
        f.render_widget(Paragraph::new("no matches").dim(), chunks[2]);
    } else {
        let items: Vec<ListItem> = rows.into_iter().map(ListItem::new).collect();
        f.render_stateful_widget(
            List::new(items).highlight_symbol("▶ "),
            chunks[2],
            &mut app.state,
        );
    }

    match status {
        Some(p) => f.render_widget(p, chunks[3]),
        None => {
            let esc = match app.stage {
                Stage::Commands => "esc to close",
                Stage::Targets { .. } => "esc to go back",
            };
            let counts = format!("{}/{} · {esc}", app.shown(), app.total());
            f.render_widget(
                Paragraph::new(footer(&counts, chunks[3].width)).dim(),
                chunks[3],
            );
        }
    }
}

/// Measured by rendering because counting characters under-counts a word-wrapped
/// message, cutting the row that names the cause; ratatui's own `line_count`
/// would answer this but sits behind an unstable feature.
fn wrapped_height(status: &Paragraph, area: Rect) -> u16 {
    if area.width == 0 || area.height == 0 {
        return 1;
    }
    let probe = Rect::new(0, 0, area.width, area.height);
    let mut buffer = Buffer::empty(probe);
    status.render(probe, &mut buffer);

    let used = (0..probe.height)
        .rev()
        .find(|&y| (0..probe.width).any(|x| buffer[(x, y)].symbol().trim() != ""))
        .map(|y| y + 1)
        .unwrap_or(1);
    used.max(1)
}

/// The counts on the left, the running version flush right. Herdr can hand the
/// plugin a region narrower than docs/design.md §5's floor, and there the counts
/// are what has to survive — so the version is dropped rather than either half
/// being truncated.
fn footer(counts: &str, width: u16) -> Line<'static> {
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    let width = width as usize;
    let gap = width
        .checked_sub(counts.chars().count() + version.chars().count())
        .filter(|gap| *gap >= 1);

    match gap {
        Some(gap) => Line::from(vec![
            Span::raw(counts.to_string()),
            Span::raw(" ".repeat(gap)),
            Span::raw(version),
        ]),
        None => Line::from(counts.to_string()),
    }
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use crate::app::Candidate;
    use crate::catalog::Command;
    use crate::frecency::Frecency;
    use crate::herdr::Target;
    use ratatui::backend::TestBackend;
    use std::path::Path;

    fn command(id: &str, title: &str, resolve: Option<&str>) -> Command {
        Command {
            id: id.to_string(),
            title: title.to_string(),
            args: vec!["noop".to_string()],
            contexts: Vec::new(),
            resolve: resolve.map(str::to_string),
        }
    }

    fn app_with(commands: Vec<Command>) -> App {
        let candidates = commands.into_iter().map(Candidate::from_command).collect();
        App::new(candidates, Frecency::load(Path::new("/nonexistent")))
    }

    /// Draws into the region Herdr hands the plugin and returns it as lines.
    fn draw(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                (0..width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    /// The regression #31 fixed: a bordered Block here landed inside Herdr's
    /// own frame, so the palette showed two.
    #[test]
    fn draws_no_frame_of_its_own() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        let lines = draw(&mut app, 36, 8);
        let border = ['┌', '┐', '└', '┘', '─', '│'];
        assert!(
            !lines.iter().any(|l| l.chars().any(|c| border.contains(&c))),
            "border drawn: {lines:#?}"
        );
    }

    /// Herdr's pane title is static, so the selected command's name has to be
    /// visible from inside the pane.
    #[test]
    fn targets_stage_names_the_selected_command() {
        let picked = command("focus.tab", "Focus tab…", Some("tabs"));
        let mut app = app_with(vec![picked.clone()]);
        app.enter_targets(
            picked,
            vec![Target {
                id: "1".into(),
                label: "editor".into(),
            }],
        );
        let lines = draw(&mut app, 36, 8);
        assert_eq!(lines[0], "Focus tab…", "{lines:#?}");
        assert_eq!(lines[1], ">", "{lines:#?}");
    }

    /// The Commands stage must not pay for the header row it has no use for.
    #[test]
    fn commands_stage_starts_at_the_query_line() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        let lines = draw(&mut app, 36, 8);
        assert_eq!(lines[0], ">", "{lines:#?}");
    }

    /// Width is the axis docs/design.md §5 gives a readability floor to, and
    /// nothing else here would notice the region narrowing: every other
    /// assertion is on a short string that fits whatever it is given.
    #[test]
    fn every_column_of_the_pane_is_drawn_into() {
        let wide = "W".repeat(80);
        let picked = command("focus.tab", &wide, Some("tabs"));
        let mut app = app_with(vec![picked.clone()]);
        app.enter_targets(
            picked,
            vec![Target {
                id: "1".into(),
                label: wide.clone(),
            }],
        );
        let lines = draw(&mut app, 36, 8);
        let overlong: Vec<&String> = lines.iter().filter(|l| l.contains('W')).collect();
        assert_eq!(overlong.len(), 2, "header and list row: {lines:#?}");
        for line in overlong {
            assert_eq!(line.chars().count(), 36, "{line:?} in {lines:#?}");
        }
    }

    /// Issue #36: the running build has to be readable from inside the palette,
    /// at the width docs/design.md §5 floors at.
    #[test]
    fn the_footer_ends_with_the_running_version() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        let lines = draw(&mut app, 36, 8);
        let footer = lines.last().unwrap();
        assert!(
            footer.starts_with("1/1 · esc to close"),
            "counts kept their place: {lines:#?}"
        );
        assert!(
            footer.ends_with(&format!("v{}", env!("CARGO_PKG_VERSION"))),
            "{lines:#?}"
        );
    }

    /// The version tracks Cargo.toml rather than a literal that a release bump
    /// would leave behind, so the assertion above cannot be a hardcoded string.
    #[test]
    fn the_version_is_the_crate_version() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        let lines = draw(&mut app, 36, 8);
        assert!(
            lines.last().unwrap().ends_with(env!("CARGO_PKG_VERSION")),
            "{lines:#?}"
        );
    }

    /// The Targets stage carries a different esc hint, and the version sits to
    /// the right of that one too.
    #[test]
    fn the_targets_stage_footer_carries_the_version() {
        let picked = command("focus.tab", "Focus tab…", Some("tabs"));
        let mut app = app_with(vec![picked.clone()]);
        app.enter_targets(
            picked,
            vec![Target {
                id: "1".into(),
                label: "editor".into(),
            }],
        );
        let lines = draw(&mut app, 36, 8);
        let footer = lines.last().unwrap();
        assert!(footer.starts_with("1/1 · esc to go back"), "{lines:#?}");
        assert!(
            footer.ends_with(&format!("v{}", env!("CARGO_PKG_VERSION"))),
            "{lines:#?}"
        );
    }

    /// A status message is the row's whole content — right-aligning the version
    /// against it would cost the message the columns it needs.
    #[test]
    fn a_status_message_hides_the_version() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        app.status = Some("Split pane: right".to_string());
        let lines = draw(&mut app, 36, 8);
        assert_eq!(lines.last().unwrap(), "Split pane: right", "{lines:#?}");
    }

    /// A dispatch failure carries herdr's own message and runs well past the
    /// popup's width. Truncated to one line it loses the half naming the cause,
    /// which leaves the user where the silent failure did — knowing only that
    /// nothing happened.

    #[test]
    fn a_dispatch_failure_is_readable_in_full() {
        let message =
            "`pane.move.tab` failed: pane cannot be moved into the tab it already occupies";
        let mut app = app_with(vec![command("pane.move.tab", "Move pane to tab", None)]);
        app.status = Some(message.to_string());

        let lines = draw(&mut app, 60, 12);
        let shown: String = lines
            .iter()
            .skip_while(|l| !l.contains("pane.move.tab` failed"))
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(shown, message, "{lines:#?}");
    }

    /// This message wraps to three rows but is 105 chars over 60 columns, so a
    /// character count gives it two and drops the row naming the cause.
    #[test]
    fn a_failure_that_word_wraps_is_not_cut_short() {
        let message = "`pane.move.tab` failed: aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let mut app = app_with(vec![command("pane.move.tab", "Move pane to tab", None)]);
        app.status = Some(message.to_string());

        let lines = draw(&mut app, 60, 12);
        let shown: String = lines
            .iter()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            shown.ends_with("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
            "the wrapped tail was cut: {lines:#?}"
        );
    }

    /// `wrapped_height` grows with the message and has no cap of its own, so
    /// what keeps a candidate visible is the layout trimming a `Length` that
    /// exceeds the pane. Verified by rendering rather than reasoned about — the
    /// trimming is ratatui's behaviour, not something this file states.
    #[test]
    fn a_long_status_leaves_the_list_a_row() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        app.status = Some("x".repeat(500));
        let lines = draw(&mut app, 36, 8);
        assert!(
            lines.iter().any(|l| l.contains("Split pane: right")),
            "the candidate was pushed off: {lines:#?}"
        );
    }

    /// Herdr can hand the plugin a region narrower than §5's floor. The counts
    /// are what has to survive there, so the version is dropped whole rather
    /// than either half being truncated.
    #[test]
    fn a_pane_too_narrow_for_both_drops_the_version() {
        let counts = "1/1 · esc to close";
        let version_width = 1 + env!("CARGO_PKG_VERSION").chars().count();
        let one_column_short = counts.chars().count() + version_width;

        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        let lines = draw(&mut app, one_column_short as u16, 8);
        assert_eq!(lines.last().unwrap(), counts, "{lines:#?}");
    }

    /// The stage that pays for the header is the one to measure, at the
    /// contracted grid docs/design.md §5 floors at min_height = 8. More
    /// candidates than can fit, so the count is the list's height.
    #[test]
    fn the_list_stays_usable_at_the_documented_height_floor() {
        let picked = command("focus.tab", "Focus tab…", Some("tabs"));
        let mut app = app_with(vec![picked.clone()]);
        app.enter_targets(
            picked,
            (0..99)
                .map(|i| Target {
                    id: i.to_string(),
                    label: format!("Target {i}"),
                })
                .collect(),
        );
        let lines = draw(&mut app, 36, 8);
        let listed = lines.iter().filter(|l| l.contains("Target")).count();
        assert_eq!(listed, 5, "{lines:#?}");
    }
}

/// Keys in, argv out. Every other test here stops at a boundary: `app::tests`
/// checks `confirm` against a state built by hand, and `render_tests` checks
/// what is drawn. Neither covers the join — that a key the user presses reaches
/// `App`, that Enter reaches `confirm`, and that what falls out is the argv
/// `Herdr::dispatch` spawns. A break anywhere along that path shows up as a
/// palette that takes the keypress and runs nothing.
#[cfg(test)]
mod wiring_tests {
    use super::*;
    use crate::app::{Candidate, Outcome};
    use crate::catalog::Command;
    use crate::frecency::Frecency;
    use crate::herdr::{PluginAction, Target};
    use crossterm::event::{KeyEvent, KeyEventState};

    fn command(id: &str, title: &str, args: &[&str], resolve: Option<&str>) -> Command {
        Command {
            id: id.to_string(),
            title: title.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            contexts: Vec::new(),
            resolve: resolve.map(str::to_string),
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

    fn press(app: &mut App, typed: &str, keys: &[KeyCode]) -> Option<Step> {
        let codes = typed.chars().map(KeyCode::Char).chain(keys.iter().copied());
        for code in codes {
            match apply(app, key(code)) {
                None | Some(Step::Continue) => {}
                Some(step) => return Some(step),
            }
        }
        None
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

    /// Every failure above is "the palette accepted the key and ran nothing",
    /// so the panic has to say what it did instead.
    fn describe(step: &Option<Step>) -> String {
        match step {
            None => "the keys were consumed and no step was produced".into(),
            Some(Step::Continue) => "Continue".into(),
            Some(Step::Cancel) => "Cancel".into(),
            Some(Step::NeedsTargets(c)) => format!("NeedsTargets({})", c.id),
            Some(Step::Run(Outcome::Command { id, args })) => {
                format!("Run({id}: {})", args.join(" "))
            }
            Some(Step::Run(Outcome::Action { id, .. })) => format!("Run(action {id})"),
        }
    }
}
