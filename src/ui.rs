//! Rendering and the event loop. The pane entrypoint has a real TTY, unlike the
//! action hop (docs/design.md §3).
use std::io::{stdout, Stdout};
use std::rc::Rc;

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
    // Herdr draws the pane's own bordered frame (title from herdr-plugin.toml);
    // a second Block here doubled it, so the Targets command name gets its own line instead.
    let chunks = match &app.stage {
        Stage::Commands => Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area()),
        Stage::Targets { command, .. } => {
            let rows = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .split(f.area());
            f.render_widget(Paragraph::new(command.title.clone()).bold(), rows[0]);
            Rc::new([rows[1], rows[2], rows[3]])
        }
    };

    f.render_widget(Paragraph::new(format!("> {}", app.query)), chunks[0]);

    // Owned rather than borrowed: the list borrows `app` immutably while
    // render_stateful_widget needs `app.state` mutably.
    let rows: Vec<String> = app.rows().into_iter().map(str::to_owned).collect();
    if rows.is_empty() {
        f.render_widget(Paragraph::new("no matches").dim(), chunks[1]);
    } else {
        let items: Vec<ListItem> = rows.into_iter().map(ListItem::new).collect();
        f.render_stateful_widget(
            List::new(items).highlight_symbol("▶ "),
            chunks[1],
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
    f.render_widget(Paragraph::new(footer).dim(), chunks[2]);
}
