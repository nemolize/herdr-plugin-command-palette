// Prototype palette: input line, filtered list, keyboard nav, herdr JSON.
// Scope-matched to prototypes/go so the two can be compared.
use std::io::stdout;
use std::process::Command;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use serde::Deserialize;

#[derive(Deserialize)]
struct PaneList {
    result: PaneResult,
}

#[derive(Deserialize)]
struct PaneResult {
    panes: Vec<Pane>,
}

#[derive(Deserialize)]
struct Pane {
    pane_id: String,
    terminal_title_stripped: Option<String>,
}

struct Candidate {
    title: String,
    id: String,
}

fn load_candidates() -> Result<Vec<Candidate>, Box<dyn std::error::Error>> {
    let out = Command::new("herdr").args(["pane", "list"]).output()?;
    let parsed: PaneList = serde_json::from_slice(&out.stdout)?;
    Ok(parsed
        .result
        .panes
        .into_iter()
        .map(|p| {
            let title = p
                .terminal_title_stripped
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| p.pane_id.clone());
            Candidate { title, id: p.pane_id }
        })
        .collect())
}

/// Subsequence match; score rewards earlier and tighter matches.
fn fuzzy(query: &str, target: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let t: Vec<char> = target.to_lowercase().chars().collect();
    let (mut qi, mut score, mut prev) = (0usize, 0i32, -1i32);
    for (ti, ch) in t.iter().enumerate() {
        if qi >= q.len() {
            break;
        }
        if *ch == q[qi] {
            if prev >= 0 && ti as i32 == prev + 1 {
                score += 5;
            }
            score -= (ti / 10) as i32;
            prev = ti as i32;
            qi += 1;
        }
    }
    (qi == q.len()).then_some(score)
}

struct App {
    all: Vec<Candidate>,
    filtered: Vec<usize>, // indices into `all`, never references
    query: String,
    state: ListState,
}

impl App {
    fn new(all: Vec<Candidate>) -> Self {
        let mut app = App { all, filtered: Vec::new(), query: String::new(), state: ListState::default() };
        app.refilter();
        app
    }

    fn refilter(&mut self) {
        let mut scored: Vec<(i32, usize)> = self
            .all
            .iter()
            .enumerate()
            .filter_map(|(i, c)| fuzzy(&self.query, &c.title).map(|s| (s * 1000 - i as i32, i)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        self.filtered = scored.into_iter().map(|(_, i)| i).collect();
        self.state.select(if self.filtered.is_empty() { None } else { Some(0) });
    }

    fn move_sel(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let cur = self.state.selected().unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, self.filtered.len() as i32 - 1);
        self.state.select(Some(next as usize));
    }

    fn chosen(&self) -> Option<&str> {
        self.state
            .selected()
            .and_then(|i| self.filtered.get(i))
            .map(|&i| self.all[i].id.as_str())
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let candidates = load_candidates()?;
    let mut app = App::new(candidates);

    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut picked: Option<String> = None;
    loop {
        terminal.draw(|f| render(f, &mut app))?;
        if let Event::Key(key) = event::read()? {
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Esc => break,
                KeyCode::Up => app.move_sel(-1),
                KeyCode::Down => app.move_sel(1),
                KeyCode::Enter => {
                    picked = app.chosen().map(str::to_owned);
                    break;
                }
                KeyCode::Backspace => {
                    app.query.pop();
                    app.refilter();
                }
                KeyCode::Char(c) => {
                    app.query.push(c);
                    app.refilter();
                }
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;
    if let Some(id) = picked {
        println!("{id}");
    }
    Ok(())
}

fn render(f: &mut Frame, app: &mut App) {
    let chunks = Layout::vertical([Constraint::Length(2), Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    f.render_widget(Paragraph::new(format!("> {}", app.query)), chunks[0]);

    let items: Vec<ListItem> = app
        .filtered
        .iter()
        .take(10)
        .map(|&i| ListItem::new(app.all[i].title.as_str()))
        .collect();
    f.render_stateful_widget(
        List::new(items).highlight_symbol("▶ "),
        chunks[1],
        &mut app.state,
    );

    f.render_widget(
        Paragraph::new(format!("{}/{} · esc to close", app.filtered.len(), app.all.len())),
        chunks[2],
    );
}
