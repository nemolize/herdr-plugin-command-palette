//! Rendering and the event loop. The pane entrypoint has a real TTY, unlike the
//! action hop (docs/design.md §3).
use std::io::{stdout, Stdout};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph};

use crate::app::{App, Stage, Step};

/// Restores the terminal on drop, so an error path cannot leave the pane in raw
/// mode with the alternate screen still up.
pub struct Screen {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Screen {
    pub fn enter() -> Result<Self, String> {
        enable_raw_mode().map_err(|e| e.to_string())?;
        stdout()
            .execute(EnterAlternateScreen)
            .map_err(|e| e.to_string())?;
        let terminal = Terminal::new(CrosstermBackend::new(stdout())).map_err(|e| e.to_string())?;
        Ok(Screen { terminal })
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
        // A resize has to redraw immediately rather than wait for a keypress:
        // on Termux the popup resizes exactly when the software keyboard is
        // raised, which is the moment the palette is being used (§5).
        if matches!(event, Event::Resize(_, _)) {
            return Ok(Step::Continue);
        }
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        // Ctrl-C is the other reflex for "get me out of here", and a palette
        // that ignored it would read as hung.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Ok(Step::Cancel);
        }
        return Ok(match key.code {
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
        });
    }
}

fn render(f: &mut Frame, app: &mut App) {
    // Herdr draws the pane's own frame; a second Block here doubled it. Its
    // title is static, so Targets shows the command name on a line of its own.
    let header = match &app.stage {
        Stage::Commands => None,
        Stage::Targets { command, .. } => Some(command.title.clone()),
    };

    let chunks = Layout::vertical([
        Constraint::Length(if header.is_some() { 1 } else { 0 }),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
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

    let footer = match &app.status {
        Some(msg) => msg.clone(),
        None => {
            let esc = match app.stage {
                Stage::Commands => "esc to close",
                Stage::Targets { .. } => "esc to go back",
            };
            format!("{}/{} · {esc}", app.shown(), app.total())
        }
    };
    f.render_widget(Paragraph::new(footer).dim(), chunks[3]);
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
