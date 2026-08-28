//! Rendering and the event loop. The pane entrypoint has a real TTY, unlike the
//! action hop (docs/design.md §3).
use std::io::{stdout, Stdout};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

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

/// Dismissal is Esc, the toggle key, or picking an entry — there is no
/// click-outside-to-dismiss, because no mouse events reach a plugin at all (§6).
pub fn next_step(app: &mut App) -> Result<Step, String> {
    loop {
        let Event::Key(key) = event::read().map_err(|e| e.to_string())? else {
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
            KeyCode::Esc => Step::Cancel,
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
            KeyCode::Char(c) => {
                app.push(c);
                Step::Continue
            }
            _ => Step::Continue,
        });
    }
}

fn render(f: &mut Frame, app: &mut App) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(match &app.stage {
            Stage::Commands => " Command Palette ".to_string(),
            Stage::Targets { command, .. } => format!(" {} ", command.title),
        });
    let inner = block.inner(f.area());
    f.render_widget(block, f.area());

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(inner);

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
        None => format!("{}/{} · esc to close", app.shown(), app.total()),
    };
    f.render_widget(Paragraph::new(footer).dim(), chunks[2]);
}
