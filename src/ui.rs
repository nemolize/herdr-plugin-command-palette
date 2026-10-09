//! Rendering and the event loop. The pane entrypoint has a real TTY, unlike the
//! action hop (docs/design.md §3).
use std::io::{stdout, Stdout};

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::buffer::CellWidth;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};
use unicode_segmentation::UnicodeSegmentation;

use crate::app::{App, Stage, Step};
use crate::generate_row::GenerateRow;
use crate::glyph::{
    control_picture, CURSOR, CURSOR_COLUMNS, HIGHLIGHT_COLUMNS, HIGHLIGHT_SYMBOL, SEPARATOR,
    STAND_IN,
};
use crate::line_edit::{Edit, LineEdit};

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

    /// Returns the height drawn at, which is what the user is looking at.
    pub fn draw(&mut self, app: &mut App) -> Result<u16, String> {
        self.terminal
            .draw(|f| render(f, app))
            .map(|frame| frame.area.height)
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

/// The height of the palette's own terminal below which it draws only the
/// too-short message: the tallest stage, Commands with a skip reason, listing
/// two candidates. Herdr's manifest has no size floor, so the palette enforces
/// it (docs/design.md §5).
const MIN_ROWS: u16 = INPUT_ROWS + FLOOR_CANDIDATES + REASON_ROWS + FOOTER_ROWS;

/// The query, or the typed name.
const INPUT_ROWS: u16 = 1;

const FOOTER_ROWS: u16 = 1;

fn render(f: &mut Frame, app: &mut App) {
    let rows = f.area().height;
    if rows < MIN_ROWS {
        let message = vec![
            Line::from(format!("Too short: needs {MIN_ROWS} rows, has {rows}")),
            Line::from("Esc to close").dim(),
        ];
        f.render_widget(Paragraph::new(message).wrap(Wrap { trim: true }), f.area());
        return;
    }

    // Herdr draws the pane's own frame; a second Block here doubled it. Its
    // title is static, so Targets shows the command name on a line of its own.
    let header = match &app.stage {
        Stage::Commands => None,
        Stage::Targets { command, .. } => Some(command.title.clone()),
        Stage::Prompt { command, .. } => Some(
            command
                .prompt
                .clone()
                .unwrap_or_else(|| command.title.clone()),
        ),
    };

    let status = app.status.as_ref().map(|msg| {
        Paragraph::new(msg.trim_end().to_string())
            .wrap(Wrap { trim: false })
            .dim()
    });
    let reserved = u16::from(header.is_some())
        + INPUT_ROWS
        + if matches!(app.stage, Stage::Prompt { .. }) {
            LIST_MIN_ROWS
        } else {
            FLOOR_CANDIDATES
        }
        + if app.selected_note().is_some() {
            REASON_ROWS
        } else {
            0
        };
    let status_room = rows.saturating_sub(reserved).clamp(1, MAX_STATUS_ROWS);
    let (status_height, status_cut) = match &status {
        Some(p) => {
            let needed = wrapped_height(p, f.area().width);
            (needed.min(status_room), needed > status_room)
        }
        None => (FOOTER_ROWS, false),
    };

    let chunks = Layout::vertical([
        Constraint::Length(if header.is_some() { 1 } else { 0 }),
        Constraint::Length(INPUT_ROWS),
        Constraint::Min(LIST_MIN_ROWS),
        Constraint::Length(status_height),
    ])
    .split(f.area());

    if let Some(title) = header {
        f.render_widget(Paragraph::new(title).bold(), chunks[0]);
    }

    if let Stage::Prompt { text, generate, .. } = &app.stage {
        let row_focused = generate.as_ref().is_some_and(|r| r.focused);
        render_input(f, text, !row_focused, chunks[1]);
        if let Some(row) = generate {
            render_generate_row(f, row, chunks[2]);
        }
        render_status(f, app, status, status_cut, chunks[3]);
        return;
    }

    render_input(f, &app.selection.query, true, chunks[1]);

    // Owned rather than borrowed: the list borrows `app` immutably while
    // render_stateful_widget needs `app.selection.state` mutably.
    let rows: Vec<String> = app.rows().into_iter().map(str::to_owned).collect();
    let keys: Vec<String> = app.row_keys().into_iter().map(str::to_owned).collect();
    let reason = app.selected_note().map(str::to_owned);
    if rows.is_empty() {
        f.render_widget(Paragraph::new("no matches").dim(), chunks[2]);
    } else {
        // Rows stay ONE line each so the list's own navigation is untouched;
        // the reason wraps in an area of its own instead of growing a row.
        let (list_area, reason_area) = match &reason {
            None => (chunks[2], None),
            Some(_) => {
                let split = Layout::vertical([Constraint::Min(1), Constraint::Length(REASON_ROWS)])
                    .split(chunks[2]);
                (split[0], Some(split[1]))
            }
        };

        let items: Vec<ListItem> = rows
            .iter()
            .zip(keys.iter())
            .map(|(title, key)| ListItem::new(row_line(title, key, list_area.width)))
            .collect();
        f.render_stateful_widget(
            List::new(items).highlight_symbol(HIGHLIGHT_SYMBOL),
            list_area,
            &mut app.selection.state,
        );
        if let (Some(area), Some(why)) = (reason_area, reason) {
            f.render_widget(Paragraph::new(why).wrap(Wrap { trim: true }).dim(), area);
        }
    }

    render_status(f, app, status, status_cut, chunks[3]);
}

const MAX_STATUS_ROWS: u16 = 5;

const LIST_MIN_ROWS: u16 = 1;

/// A status message never takes these.
const FLOOR_CANDIDATES: u16 = 2;

const REASON_ROWS: u16 = 2;

/// ASCII rather than `…`, which is East Asian Ambiguous (see `glyph`).
const CUT_MARKER: &str = "...";

fn render_status(f: &mut Frame, app: &App, status: Option<Paragraph>, cut: bool, area: Rect) {
    match status {
        Some(p) => {
            f.render_widget(p, area);
            if cut {
                mark_cut(f.buffer_mut(), area);
            }
        }
        None => render_footer(f, app, area),
    }
}

fn mark_cut(buffer: &mut Buffer, area: Rect) {
    if area.is_empty() {
        return;
    }
    let y = area.bottom() - 1;
    let marker_width = CUT_MARKER.len() as u16;
    let text_end = (area.left()..area.right())
        .filter(|&x| buffer[(x, y)].symbol().trim() != "")
        .map(|x| x + buffer[(x, y)].symbol().cell_width().max(1))
        .max()
        .unwrap_or(area.left());
    let x = text_end.min(area.right().saturating_sub(marker_width).max(area.left()));
    // The cell left of the marker may hold a wide glyph whose second half the
    // marker now covers; a half-drawn glyph would overprint the marker.
    if x > area.left() && buffer[(x - 1, y)].symbol().cell_width() > 1 {
        buffer[(x - 1, y)].reset();
    }
    let style = buffer[(x.min(area.right() - 1), y)].style();
    buffer.set_stringn(x, y, CUT_MARKER, (area.right() - x) as usize, style);
}

/// Columns the `> ` prefix takes from the input line.
const PROMPT_COLUMNS: u16 = 2;

/// Blank columns between a title and the key flush right, so the two read as
/// separate columns rather than one run-on string.
const KEY_GAP: u16 = 2;

/// The key is dropped whole rather than clipped, as §5's footer drops the
/// version: a half-written chord is a chord that does not work.
fn row_line(title: &str, key: &str, width: u16) -> Line<'static> {
    let without_key = || Line::from(title.to_string());
    if key.is_empty() || width == 0 {
        return without_key();
    }

    // The highlight symbol indents every row, so the columns a row may use are
    // fewer than the area's own width and a key sized against it would wrap.
    let Some(room) = width.checked_sub(HIGHLIGHT_COLUMNS) else {
        return without_key();
    };
    let key_width = drawn_width(key);
    if room < drawn_width(title) + KEY_GAP + key_width {
        return without_key();
    }
    // ratatui draws a control character as nothing, yet `Span::width` counts it.
    let shown: String = title.chars().filter(|c| !c.is_control()).collect();
    // `Line` starts a span at the previous `Span::width`, a cell short after `ｶﾞ`,
    // so a gap span of its own would overwrite the title's last glyph.
    let next_span_column = Span::raw(shown.as_str()).width();
    let Some(gap) = usize::from(room - key_width).checked_sub(next_span_column) else {
        return without_key();
    };

    Line::from(vec![
        Span::raw(format!("{shown}{}", " ".repeat(gap))),
        Span::raw(key.to_string()).dim(),
    ])
}

/// Indented the same whether or not it has focus, so the label does not jump.
/// A failure reason too long for the row is marked cut like a long status.
fn render_generate_row(f: &mut Frame, row: &GenerateRow, area: Rect) {
    let marker = if row.focused {
        HIGHLIGHT_SYMBOL.to_string()
    } else {
        " ".repeat(usize::from(HIGHLIGHT_COLUMNS))
    };
    let text = format!("{marker}{}", row.label());
    let cut = drawn_width(&text) > area.width;
    let line = Line::from(text);
    let line = if row.is_generating() {
        line.dim()
    } else {
        line
    };
    let area = Rect { height: 1, ..area };
    f.render_widget(Paragraph::new(line), area);
    if cut {
        mark_cut(f.buffer_mut(), area);
    }
}

/// Draws the query or the typed name with the cursor in it (docs/design.md §4).
///
/// The cursor is reserved a cell BEFORE the text is measured, which is what
/// makes clipping it away with the text unreachable rather than a calculation
/// to keep honest. The text before the cursor claims the room first, so the
/// cursor and what was just typed stay on screen.
///
/// An unfocused line keeps the cursor's cell blank rather than dropping it, so
/// the text does not shift when focus moves.
fn render_input(f: &mut Frame, line: &LineEdit, focused: bool, area: Rect) {
    if area.width == 0 {
        return;
    }
    let (before, after) = line.split();
    let (before, after) = (drawn_clusters(before), drawn_clusters(after));
    // The cursor outranks the prefix when the pane cannot hold both: it is what
    // says the field is live, and a lone `>` says nothing.
    let cursor_column = match area.width.checked_sub(PROMPT_COLUMNS + CURSOR_COLUMNS) {
        None => 0,
        Some(room) => {
            let kept = tail_within(&before, room);
            f.render_widget(Paragraph::new("> "), area);
            render_clusters(f, kept, area.x + PROMPT_COLUMNS, area);
            PROMPT_COLUMNS + clusters_width(kept)
        }
    };

    let cursor = Rect {
        x: area.x + cursor_column,
        width: CURSOR_COLUMNS,
        ..area
    };
    f.render_widget(Paragraph::new(if focused { CURSOR } else { " " }), cursor);
    let rest = cursor_column + CURSOR_COLUMNS;
    let room = area.width.saturating_sub(rest);
    render_clusters(f, head_within(&after, room), area.x + rest, area);
}

/// The drawn form of each cluster of `text`, the clusters `LineEdit` puts its
/// cursor stops between, so every stop falls between two drawn glyphs.
fn drawn_clusters(text: &str) -> Vec<String> {
    text.graphemes(true).map(drawn_cluster).collect()
}

/// `cluster` with controls as Control Pictures (`CellWidth` panics on a raw one
/// in a debug build), and on `STAND_IN` if it would draw nothing at all.
fn drawn_cluster(cluster: &str) -> String {
    let drawn: String = cluster
        .chars()
        .map(|c| {
            if c.is_control() {
                control_picture(c)
            } else {
                c
            }
        })
        .collect();
    if drawn_width(&drawn) > 0 {
        return drawn;
    }
    // A format character such as U+200B is a cluster break of its own, so it
    // would not ride on the stand-in the way a mark does; it gives way to it.
    let based = format!("{STAND_IN}{drawn}");
    if based.graphemes(true).count() == 1 {
        based
    } else {
        STAND_IN.to_string()
    }
}

/// Each cluster at the column `drawn_width` puts it in, not one `Line` of spans:
/// that places spans by `Span::width`, a cell short after `ｶﾞ`.
fn render_clusters(f: &mut Frame, clusters: &[String], x: u16, area: Rect) {
    let mut x = x;
    for cluster in clusters {
        let width = drawn_width(cluster);
        f.render_widget(Span::raw(cluster.as_str()), Rect { x, width, ..area });
        x += width;
    }
}

fn clusters_width(clusters: &[String]) -> u16 {
    clusters.iter().map(|c| drawn_width(c)).sum()
}

/// Cells `text` occupies once drawn, per ratatui's own per-cell measurement.
///
/// `Line::width` is NOT this number — it reports 1 for `ｶ\u{FF9E}`, which the
/// buffer lays out in 2. Summing scalars is wrong too, in both directions.
fn drawn_width(text: &str) -> u16 {
    // Dropped rather than measured because `CellWidth` panics on one in a debug
    // build, and a catalog title is user-written.
    text.graphemes(true)
        .filter(|cluster| !cluster.chars().any(char::is_control))
        .map(|cluster| cluster.cell_width())
        .sum()
}

/// The last of `clusters` that fit in `room` columns. Whole clusters only:
/// cutting inside one draws a DIFFERENT glyph, or `␊` without its `␍`.
fn tail_within(clusters: &[String], room: u16) -> &[String] {
    let mut used = 0;
    let kept = clusters
        .iter()
        .rev()
        .take_while(|c| {
            used += drawn_width(c);
            used <= room
        })
        .count();
    &clusters[clusters.len() - kept..]
}

fn head_within(clusters: &[String], room: u16) -> &[String] {
    let mut used = 0;
    let kept = clusters
        .iter()
        .take_while(|c| {
            used += drawn_width(c);
            used <= room
        })
        .count();
    &clusters[..kept]
}

/// The counts row. Only reached when no status message has claimed the row.
fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let esc = match app.stage {
        Stage::Commands => "esc to close",
        Stage::Targets { .. } | Stage::Prompt { .. } => "esc to go back",
    };
    // The typing stage lists nothing, so counts there would read 0/0.
    let counts = match app.generate_row() {
        Some(row) if row.is_generating() => "esc to cancel".to_string(),
        Some(row) if row.focused => format!("enter to generate{SEPARATOR}{esc}"),
        _ if matches!(app.stage, Stage::Prompt { .. }) => format!("enter to run{SEPARATOR}{esc}"),
        _ => format!("{}/{}{SEPARATOR}{esc}", app.shown(), app.total()),
    };
    f.render_widget(Paragraph::new(footer(&counts, area.width)).dim(), area);
}

/// Counted rather than rendered into a probe: a probe only as tall as the pane
/// misses text after a longer run of blank lines (#151).
fn wrapped_height(status: &Paragraph, width: u16) -> u16 {
    u16::try_from(status.line_count(width))
        .unwrap_or(u16::MAX)
        .max(1)
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
    use crate::listing::Target;
    use ratatui::backend::TestBackend;
    use std::path::Path;

    fn command(id: &str, title: &str, resolve: Option<&str>) -> Command {
        Command {
            id: id.to_string(),
            title: title.to_string(),
            args: vec!["noop".to_string()],
            contexts: Vec::new(),
            resolve: resolve.map(str::to_string),
            prompt: None,
            binding: None,
            retired_icon: None,
        }
    }

    fn app_with(commands: Vec<Command>) -> App {
        let candidates = commands.into_iter().map(Candidate::from_command).collect();
        App::new(candidates, Frecency::load(Path::new("/nonexistent")))
    }

    /// One entry carrying the key it is also reachable by.
    fn app_with_key(title: &str, key: &str) -> App {
        let mut entry = command("entry", title, None);
        entry.args = vec!["tab".into(), "create".into()];
        let mut candidate = Candidate::from_command(entry);
        candidate.key = key.to_string();
        App::new(vec![candidate], Frecency::load(Path::new("/nonexistent")))
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

    /// Issue #52: the key is what the row teaches, and it is only legible flush
    /// right, clear of a title whose length varies row to row.
    #[test]
    fn an_entrys_key_is_shown_flush_right_of_its_title() {
        let mut app = app_with_key("New tab", "prefix+c");
        let lines = draw(&mut app, 36, 8);
        let row = lines.iter().find(|l| l.contains("New tab")).unwrap();
        assert!(row.starts_with("▸ New tab"), "{lines:#?}");
        assert!(row.ends_with("prefix+c"), "{lines:#?}");
        assert_eq!(row.chars().count(), 36, "{row:?} in {lines:#?}");
    }

    /// An entry nothing binds gets no column at all — a blank one would still
    /// pad the row and read as a key that failed to render.
    #[test]
    fn an_entry_with_no_key_is_drawn_as_the_title_alone() {
        let mut app = app_with_key("New tab", "");
        let lines = draw(&mut app, 36, 8);
        let row = lines.iter().find(|l| l.contains("New tab")).unwrap();
        assert_eq!(row, "▸ New tab", "{lines:#?}");

        // The drawn row cannot tell padding from absence — a buffer trims its
        // trailing blanks either way — so the line's own spans are the oracle.
        assert_eq!(row_line("New tab", "", 36).spans.len(), 1);
    }

    /// §5's footer drops the version rather than truncate the counts, and the
    /// key answers to the same rule: a half-drawn chord is not a chord, while
    /// the title is what the user is searching by.
    #[test]
    fn a_row_too_narrow_for_both_drops_the_key_not_the_title() {
        let mut app = app_with_key("Rename workspace...", "prefix+shift+w");
        let lines = draw(&mut app, 24, 8);
        let row = lines.iter().find(|l| l.contains("Rename")).unwrap();
        assert!(!row.contains("prefix"), "key survived a clip: {lines:#?}");
        assert!(row.starts_with("▸ Rename workspace..."), "{lines:#?}");
    }

    /// A catalog is user-editable and a plugin's title is another author's, so
    /// a control char reaches this measurement — where `CellWidth` panics in a
    /// debug build. `render_input` draws them as pictures for the same reason.
    #[test]
    fn a_control_char_in_a_title_is_measured_rather_than_panicking() {
        let line = row_line("New\ttab", "prefix+c", 36);
        assert!(!line.spans.is_empty());
    }

    /// Issue #110: every row is laid out against `HIGHLIGHT_COLUMNS`, so a marker
    /// a CJK-locale terminal draws wider shifts the selected row alone.
    #[test]
    fn the_highlight_symbol_is_drawn_in_its_columns() {
        assert_eq!(drawn_width(HIGHLIGHT_SYMBOL), HIGHLIGHT_COLUMNS);
    }

    /// The key must clear the title by the full gap rather than abut it, or the
    /// two read as one string at exactly the width where the row is fullest.
    #[test]
    fn a_key_that_only_just_fits_still_clears_the_title() {
        // `▸ ` + `New tab` + the gap + `prefix+c`.
        let exact = 2 + 7 + 2 + 8;

        let mut app = app_with_key("New tab", "prefix+c");
        let lines = draw(&mut app, exact, 8);
        let row = lines.iter().find(|l| l.contains("New tab")).unwrap();
        assert_eq!(row, "▸ New tab  prefix+c", "{lines:#?}");

        let mut app = app_with_key("New tab", "prefix+c");
        let lines = draw(&mut app, exact - 1, 8);
        let row = lines.iter().find(|l| l.contains("New tab")).unwrap();
        assert_eq!(row, "▸ New tab", "{lines:#?}");
    }

    /// Issue #230: `ｶﾞ` takes two cells but one `Span::width`, and the cell it
    /// was short by used to go to the gap, over the title's last glyph.
    #[test]
    fn a_title_with_a_sound_mark_keeps_every_glyph_beside_its_key() {
        let width = 30;
        let mut app = app_with_key("aｶﾞb", "prefix+c");
        let row = cells(&mut app, width, 8, 1);
        assert_eq!(&row[2..4], ["a", "ｶﾞ"], "{row:?}");
        assert_eq!(row[5], "b", "{row:?}");
        assert_eq!(row[width as usize - 8..].concat(), "prefix+c", "{row:?}");
    }

    /// The key is dropped by drawn width, which counts `ｶﾞ` as the two cells
    /// it takes: `▸ ` + `aｶﾞb` + the gap + `prefix+c`.
    #[test]
    fn a_key_beside_a_sound_mark_is_dropped_by_drawn_width() {
        let exact = 2 + 4 + 2 + 8;

        let mut app = app_with_key("aｶﾞb", "prefix+c");
        let row = cells(&mut app, exact, 8, 1);
        assert_eq!(row[5], "b", "{row:?}");
        assert_eq!(&row[6..8], [" ", " "], "{row:?}");
        assert_eq!(row[8..].concat(), "prefix+c", "{row:?}");

        let mut app = app_with_key("aｶﾞb", "prefix+c");
        let row = cells(&mut app, exact - 1, 8, 1);
        assert_eq!(row[5], "b", "{row:?}");
        assert!(!row.concat().contains("prefix"), "{row:?}");
    }

    /// A control character draws nothing but counts a cell to `Span::width`, so
    /// left in the title it pushed a key that fits by drawn width off the row.
    #[test]
    fn a_key_beside_control_characters_is_dropped_by_drawn_width() {
        // `▸ ` + `ab` + the gap + `prefix+c`.
        let exact = 2 + 2 + 2 + 8;

        let mut app = app_with_key("a\t\t\tb", "prefix+c");
        let row = cells(&mut app, exact, 8, 1);
        assert_eq!(row[..4].concat(), "▸ ab", "{row:?}");
        assert_eq!(row[6..].concat(), "prefix+c", "{row:?}");

        let mut app = app_with_key("a\t\t\tb", "prefix+c");
        let row = cells(&mut app, exact - 1, 8, 1);
        assert!(!row.concat().contains("prefix"), "{row:?}");
    }

    /// A target has no shortcut of its own, so the stage that picks one must
    /// not carry the keys of the command list it came from.
    #[test]
    fn the_targets_stage_shows_no_keys() {
        let picked = command("focus.tab", "Focus tab...", Some("tabs"));
        let mut candidate = Candidate::from_command(picked.clone());
        candidate.key = "prefix+shift+t".to_string();
        let mut app = App::new(vec![candidate], Frecency::load(Path::new("/nonexistent")));
        app.enter_targets(
            picked,
            vec![Target {
                id: "1".into(),
                label: "editor".into(),
            }],
        );
        let lines = draw(&mut app, 36, 8);
        assert!(
            !lines.iter().any(|l| l.contains("prefix")),
            "a key reached the target list: {lines:#?}"
        );
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
        let picked = command("focus.tab", "Focus tab...", Some("tabs"));
        let mut app = app_with(vec![picked.clone()]);
        app.enter_targets(
            picked,
            vec![Target {
                id: "1".into(),
                label: "editor".into(),
            }],
        );
        let lines = draw(&mut app, 36, 8);
        assert_eq!(lines[0], "Focus tab...", "{lines:#?}");
        assert_eq!(lines[1], "> ⎸", "{lines:#?}");
    }

    fn entry(id: &str, title: &str, args: &[&str]) -> Command {
        let mut c = command(id, title, None);
        c.args = args.iter().map(|a| a.to_string()).collect();
        c
    }

    fn cells(app: &mut App, width: u16, height: u16, y: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..width)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect()
    }

    /// Issue #247: no kind of row leads with a glyph of its own, so every title
    /// starts right after the selection marker. Expectations are literal cells,
    /// never computed from the code under test.
    #[test]
    fn every_kind_of_row_starts_its_title_after_the_marker() {
        let action = crate::herdr::PluginAction {
            plugin_id: "reviewr".into(),
            action_id: "open".into(),
            title: "Review".into(),
            contexts: vec![],
            platforms: vec![],
        };
        let candidates = vec![
            Candidate::from_command(entry("p", "Pane", &["pane", "zoom"])),
            Candidate::from_command(entry("t", "Tab", &["tab", "create"])),
            Candidate::from_command(entry("w", "Work", &["workspace", "create"])),
            Candidate::from_command(entry("s", "Srv", &["server", "reload-config"])),
            Candidate::from_command(entry("k", "Tree", &["worktree", "list"])),
            Candidate::from_action(action),
            Candidate::note("bad", "why"),
        ];
        let mut app = App::new(candidates, Frecency::load(Path::new("/nonexistent")));

        let expected: [[&str; 5]; 7] = [
            ["▸", " ", "P", "a", "n"],
            [" ", " ", "T", "a", "b"],
            [" ", " ", "W", "o", "r"],
            [" ", " ", "S", "r", "v"],
            [" ", " ", "T", "r", "e"],
            [" ", " ", "R", "e", "v"],
            [" ", " ", "s", "k", "i"],
        ];
        for (row, want) in expected.iter().enumerate() {
            let got = cells(&mut app, 5, 12, 1 + row as u16);
            assert_eq!(got, want, "row {row}");
        }
    }

    /// At §5's ~36-column floor the title clips whole clusters only, so a wide
    /// glyph that does not fit leaves a blank, not half of itself.
    #[test]
    fn a_narrow_row_clips_the_title_whole() {
        let title = format!("a{}", "あ".repeat(20));
        let mut app = app_with(vec![entry("p", &title, &["pane", "zoom"])]);
        let row = cells(&mut app, 36, 8, 1);

        let mut want = vec!["▸", " ", "a"];
        for _ in 0..16 {
            want.extend(["あ", " "]);
        }
        want.push(" ");
        assert_eq!(row, want);
    }

    /// The Commands stage must not pay for the header row it has no use for.
    #[test]
    fn commands_stage_starts_at_the_query_line() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        let lines = draw(&mut app, 36, 8);
        assert_eq!(lines[0], "> ⎸", "{lines:#?}");
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
            footer.starts_with("1/1 ⋅ esc to close"),
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
        let picked = command("focus.tab", "Focus tab...", Some("tabs"));
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
        assert!(footer.starts_with("1/1 ⋅ esc to go back"), "{lines:#?}");
        assert!(
            footer.ends_with(&format!("v{}", env!("CARGO_PKG_VERSION"))),
            "{lines:#?}"
        );
    }

    /// Herdr's pane title is static, so the prompt's own label is the only place
    /// the user can read what the input is asking for.
    #[test]
    fn the_prompt_stage_names_what_it_is_asking_for() {
        let mut picked = command("tab.rename", "Rename tab...", None);
        picked.args = vec![
            "tab".into(),
            "rename".into(),
            "w3Y:t1".into(),
            "{text}".into(),
        ];
        picked.prompt = Some("New tab name".into());
        let mut app = app_with(vec![picked.clone()]);
        app.enter_prompt(picked, "editor".into());

        let lines = draw(&mut app, 36, 8);
        assert_eq!(lines[0], "New tab name", "{lines:#?}");
        assert!(lines[1].starts_with("> editor"), "seeded: {lines:#?}");
        assert!(
            lines
                .last()
                .unwrap()
                .starts_with("enter to run ⋅ esc to go back"),
            "{lines:#?}"
        );
    }

    fn renaming_workspace() -> App {
        let mut picked = command("workspace.rename", "Rename workspace...", None);
        picked.args = vec![
            "workspace".into(),
            "rename".into(),
            "w3Y".into(),
            "{text}".into(),
        ];
        picked.prompt = Some("New workspace name".into());
        let mut app = app_with(vec![picked.clone()]);
        app.enter_prompt(picked, "editor".into());
        app
    }

    /// `draw` reads the second column of the two-column `\u{2728}` as a space,
    /// hence the two spaces after it in these expectations.
    #[test]
    fn a_workspace_rename_shows_the_generate_row_under_the_input() {
        let mut app = renaming_workspace();
        let lines = draw(&mut app, 40, 8);
        assert_eq!(lines[1].trim_end(), "> editor\u{23B8}", "{lines:#?}");
        assert_eq!(
            lines[2].trim_end(),
            "  \u{2728}  Auto generate",
            "{lines:#?}"
        );
        assert!(lines[7].starts_with("enter to run"), "{lines:#?}");
    }

    #[test]
    fn the_focused_row_is_marked_and_the_input_loses_its_cursor() {
        let mut app = renaming_workspace();
        app.move_selection(1);
        let lines = draw(&mut app, 40, 8);
        assert_eq!(lines[1].trim_end(), "> editor", "{lines:#?}");
        assert_eq!(
            lines[2].trim_end(),
            "\u{25B8} \u{2728}  Auto generate",
            "{lines:#?}"
        );
        assert!(
            lines[7].starts_with("enter to generate \u{22C5} esc to go back"),
            "{lines:#?}"
        );
    }

    #[test]
    fn a_running_generation_says_so_and_how_to_cancel_it() {
        let mut app = renaming_workspace();
        app.move_selection(1);
        assert!(matches!(app.confirm(), Step::Generate { .. }));
        let lines = draw(&mut app, 40, 8);
        assert_eq!(
            lines[2].trim_end(),
            "\u{25B8} \u{2728}  Generating...",
            "{lines:#?}"
        );
        assert!(lines[7].starts_with("esc to cancel"), "{lines:#?}");
    }

    #[test]
    fn a_failure_reason_too_long_for_the_row_is_marked_cut() {
        let mut app = renaming_workspace();
        app.move_selection(1);
        let Step::Generate { id, .. } = app.confirm() else {
            panic!("expected a generate request");
        };
        app.generated(
            id,
            Err("HTTP 500: a reason far longer than the row".into()),
            std::time::Duration::ZERO,
        );
        let lines = draw(&mut app, 40, 8);
        assert!(lines[2].ends_with("..."), "{lines:#?}");
        assert_eq!(lines[2].chars().count(), 40, "fills the row: {lines:#?}");

        let mut app = renaming_workspace();
        app.move_selection(1);
        let Step::Generate { id, .. } = app.confirm() else {
            panic!("expected a generate request");
        };
        app.generated(id, Err("empty answer".into()), std::time::Duration::ZERO);
        let lines = draw(&mut app, 40, 8);
        assert!(lines[2].ends_with("empty answer"), "{lines:#?}");
    }

    #[test]
    fn tab_and_pane_renames_draw_the_generate_row_too() {
        for subject in ["tab", "pane"] {
            let mut picked = command(&format!("{subject}.rename"), "Rename...", None);
            picked.args = vec![
                subject.into(),
                "rename".into(),
                "x1".into(),
                "{text}".into(),
            ];
            let mut app = app_with(vec![picked.clone()]);
            app.enter_prompt(picked, "editor".into());
            let lines = draw(&mut app, 40, 8);
            assert_eq!(
                lines[2].trim_end(),
                "  \u{2728}  Auto generate",
                "{subject}: {lines:#?}"
            );
        }
    }

    #[test]
    fn a_prompt_that_is_not_a_rename_draws_no_generate_row() {
        let mut picked = command("tab.label", "Label tab...", None);
        picked.args = vec!["tab".into(), "label".into(), "t1".into(), "{text}".into()];
        let mut app = app_with(vec![picked.clone()]);
        app.enter_prompt(picked, "editor".into());
        let lines = draw(&mut app, 40, 8);
        assert!(
            !lines.iter().any(|l| l.contains("Auto generate")),
            "{lines:#?}"
        );
    }

    /// Expectations are FIXED cell coordinates and symbols: keep them literal.
    /// An expectation computed from the code under test cannot fail when that
    /// code under-counts, which is how two earlier versions of this test passed
    /// while the bug was live.
    #[test]
    fn the_input_line_lands_where_it_is_expected() {
        let cases: &[(&str, &str, u16, &[&str])] = &[
            (
                "short name",
                "ab",
                8,
                &[">", " ", "a", "b", "⎸", " ", " ", " "],
            ),
            (
                "clipped to the tail, cursor after it",
                "abcdef",
                6,
                &[">", " ", "d", "e", "f", "⎸"],
            ),
            (
                "a wide glyph owns two cells, the second its continuation",
                "あい",
                8,
                &[">", " ", "あ", " ", "い", " ", "⎸", " "],
            ),
            (
                "clipped between wide glyphs, never inside one",
                "あいう",
                6,
                &[">", " ", "う", " ", "⎸", " "],
            ),
            (
                "one cluster of width 2 — the case Line::width reports as 1",
                "ｶﾞ",
                6,
                &[">", " ", "ｶﾞ", " ", "⎸", " "],
            ),
            (
                "text after a cluster ratatui counts wider than unicode-width does",
                "ｶﾞa",
                8,
                &[">", " ", "ｶﾞ", " ", "a", "⎸", " ", " "],
            ),
            (
                "a lone halfwidth sound mark is drawn",
                "\u{FF9E}a",
                8,
                &[">", " ", "\u{FF9E}", "a", "⎸", " ", " ", " "],
            ),
            (
                "the cursor glyph typed as a name is still just text",
                "⎸⎸",
                8,
                &[">", " ", "⎸", "⎸", "⎸", " ", " ", " "],
            ),
            (
                "a combining mark rides with the letter it sits on",
                "e\u{301}",
                8,
                &[">", " ", "e\u{301}", "⎸", " ", " ", " ", " "],
            ),
            (
                "a variation selector rides with its base — round 5's U+FE01",
                "\u{2018}\u{FE01}",
                8,
                &[">", " ", "‘\u{FE01}", " ", "⎸", " ", " ", " "],
            ),
            (
                "a ZWJ sequence is one cluster — round 4's family emoji",
                "\u{1F469}\u{200D}\u{1F4BB}",
                8,
                &[">", " ", "👩\u{200D}💻", " ", "⎸", " ", " ", " "],
            ),
            (
                "a skin tone rides with its base — round 4's bare swatch",
                "\u{1F44D}\u{1F3FD}",
                8,
                &[">", " ", "👍🏽", " ", "⎸", " ", " ", " "],
            ),
            (
                "a flag is one cluster, never half a letter",
                "\u{1F1EF}\u{1F1F5}",
                8,
                &[">", " ", "🇯🇵", " ", "⎸", " ", " ", " "],
            ),
            (
                "emoji presentation is drawn wide — round 3's U+FE0F",
                "\u{2764}\u{FE0F}",
                8,
                &[">", " ", "❤\u{FE0F}", " ", "⎸", " ", " ", " "],
            ),
            (
                "a control character is drawn as its picture",
                "a\tb",
                8,
                &[">", " ", "a", "␉", "b", "⎸", " ", " "],
            ),
            (
                "a zero-width format character is drawn as a stand-in",
                "a\u{200B}",
                8,
                &[">", " ", "a", "◌", "⎸", " ", " ", " "],
            ),
            (
                "a mark after a control character is drawn apart from its picture",
                "\t\u{301}",
                8,
                &[">", " ", "␉", "◌\u{301}", "⎸", " ", " ", " "],
            ),
            (
                "a spacing mark after a control character keeps a cell of its own",
                "\t\u{903}",
                8,
                &[">", " ", "␉", "\u{903}", "⎸", " ", " ", " "],
            ),
            (
                "a prepended character before a control character keeps a cell of its own",
                "\u{600}\t",
                8,
                &[">", " ", "\u{600}", "␉", "⎸", " ", " ", " "],
            ),
            (
                "a CR LF pair is one cluster drawn in two cells",
                "a\r\n",
                8,
                &[">", " ", "a", "␍", "␊", "⎸", " ", " "],
            ),
        ];

        for (what, name, width, expected) in cases {
            let mut picked = command("tab.rename", "Rename tab...", None);
            picked.args = vec!["tab".into(), "rename".into(), "t1".into(), "{text}".into()];
            picked.prompt = Some("N".into());
            let mut app = app_with(vec![picked.clone()]);
            app.enter_prompt(picked, name.to_string());

            let mut terminal = Terminal::new(TestBackend::new(*width, 8)).unwrap();
            terminal.draw(|f| render(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer();
            let row: Vec<&str> = (0..*width).map(|x| buffer[(x, 1)].symbol()).collect();

            assert_eq!(&row, expected, "{what}: {name:?} at width {width}");
        }
    }

    /// Literal cells, as above. The text before the cursor claims the room
    /// first; what follows fills whatever is left.
    #[test]
    fn a_cursor_inside_the_text_is_drawn_where_it_stands() {
        let cases: &[(&str, &str, usize, u16, &[&str])] = &[
            (
                "between letters",
                "abcd",
                2,
                8,
                &[">", " ", "a", "b", "⎸", "c", "d", " "],
            ),
            (
                "at the start of a name too long to show",
                "abcdef",
                6,
                6,
                &[">", " ", "⎸", "a", "b", "c"],
            ),
            (
                "the text before it wins the room",
                "abcdef",
                2,
                6,
                &[">", " ", "b", "c", "d", "⎸"],
            ),
            (
                "a wide glyph after it is dropped whole, never halved",
                "あいう",
                1,
                8,
                &[">", " ", "あ", " ", "い", " ", "⎸", " "],
            ),
            (
                "text after a wide-counted cluster past the cursor",
                "aｶﾞb",
                2,
                8,
                &[">", " ", "a", "⎸", "ｶﾞ", " ", "b", " "],
            ),
            (
                "a control character is a visible cursor stop",
                "a\tb",
                1,
                8,
                &[">", " ", "a", "␉", "⎸", "b", " ", " "],
            ),
            (
                "a combining mark with no base after it rides on a stand-in",
                "\u{301}ab",
                3,
                8,
                &[">", " ", "⎸", "◌\u{301}", "a", "b", " ", " "],
            ),
            (
                "a mark split from a control character stays drawn",
                "\t\u{301}",
                1,
                8,
                &[">", " ", "␉", "⎸", "◌\u{301}", " ", " ", " "],
            ),
        ];

        for (what, name, left, width, expected) in cases {
            let mut picked = command("tab.rename", "Rename tab...", None);
            picked.args = vec!["tab".into(), "rename".into(), "t1".into(), "{text}".into()];
            picked.prompt = Some("N".into());
            let mut app = app_with(vec![picked.clone()]);
            app.enter_prompt(picked, name.to_string());
            for _ in 0..*left {
                app.edit(Edit::Left);
            }

            let mut terminal = Terminal::new(TestBackend::new(*width, 8)).unwrap();
            terminal.draw(|f| render(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer();
            let row: Vec<&str> = (0..*width).map(|x| buffer[(x, 1)].symbol()).collect();

            assert_eq!(
                &row, expected,
                "{what}: {name:?}, {left} left, width {width}"
            );
        }
    }

    /// Issue #224: a format character is its own cluster, so a cursor stop each
    /// side of it, and the two must not draw the same row.
    #[test]
    fn a_zero_width_format_character_is_a_visible_cursor_stop() {
        for format in ['\u{200B}', '\u{FEFF}', '\u{200E}'] {
            let name = format!("a{format}b");
            assert_eq!(
                input_row(&name, 1, 8),
                [">", " ", "a", "◌", "⎸", "b", " ", " "],
                "{name:?}"
            );
            assert_eq!(
                input_row(&name, 2, 8),
                [">", " ", "a", "⎸", "◌", "b", " ", " "],
                "{name:?}"
            );
        }
    }

    /// `\r\n` is one cursor stop drawn in two cells; clipping it in half on
    /// either side of the cursor would show one picture for the whole stop.
    #[test]
    fn a_cr_lf_pair_is_kept_or_clipped_whole() {
        for (name, left) in [("ab\r\n", 0), ("\r\nab", 4), ("a\r\nb", 1), ("a\r\nb", 2)] {
            for width in 1..=8 {
                let row = input_row(name, left, width).concat();
                assert_eq!(
                    row.contains('␍'),
                    row.contains("␍␊"),
                    "{name:?}, {left} left, width {width}: {row:?}"
                );
                assert_eq!(
                    row.contains('␊'),
                    row.contains("␍␊"),
                    "{name:?}, {left} left, width {width}: {row:?}"
                );
            }
        }
    }

    fn input_row(name: &str, left: usize, width: u16) -> Vec<String> {
        let mut picked = command("tab.rename", "Rename tab...", None);
        picked.args = vec!["tab".into(), "rename".into(), "t1".into(), "{text}".into()];
        picked.prompt = Some("N".into());
        let mut app = app_with(vec![picked.clone()]);
        app.enter_prompt(picked, name.to_string());
        for _ in 0..left {
            app.edit(Edit::Left);
        }

        let mut terminal = Terminal::new(TestBackend::new(width, 8)).unwrap();
        terminal.draw(|f| render(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..width)
            .map(|x| buffer[(x, 1)].symbol().to_string())
            .collect()
    }

    /// The cursor is what says the field is live, so it outranks the prefix when
    /// the pane is too narrow for both.
    #[test]
    fn a_pane_too_narrow_for_the_prefix_still_shows_a_cursor() {
        for width in 1..=3u16 {
            let mut picked = command("tab.rename", "Rename tab...", None);
            picked.args = vec!["tab".into(), "rename".into(), "t1".into(), "{text}".into()];
            picked.prompt = Some("N".into());
            let mut app = app_with(vec![picked.clone()]);
            app.enter_prompt(picked, "abc".into());

            let mut terminal = Terminal::new(TestBackend::new(width, 8)).unwrap();
            terminal.draw(|f| render(f, &mut app)).unwrap();
            let buffer = terminal.backend().buffer();
            let row: String = (0..width).map(|x| buffer[(x, 1)].symbol()).collect();

            assert!(row.contains('⎸'), "width {width} drew no cursor: {row:?}");
        }
    }

    /// The reason has to be READABLE, not merely stored: two earlier attempts
    /// put it somewhere one line wide and cut it mid-word. Asserted against the
    /// painted buffer, with every word of the longest real reason present.
    #[test]
    fn a_skipped_entrys_reason_is_readable_in_full() {
        let why = "`resolve` with 0 `{}` placeholders, expected 1";
        let mut app = App::new(
            vec![Candidate::note("workspace.rename", why)],
            Frecency::load(Path::new("/nonexistent")),
        );
        app.move_selection(0);

        let painted = draw(&mut app, 36, 8).join(" ");
        for word in why.split_whitespace() {
            assert!(
                painted.contains(word),
                "{word:?} was cut from the reason: {painted:?}"
            );
        }
    }

    /// The typing stage lists nothing, so the counts the other stages show would
    /// read `0/0` here — an empty result rather than a stage with no list.
    #[test]
    fn the_prompt_stage_shows_no_counts() {
        let mut picked = command("pane.rename", "Rename pane...", None);
        picked.args = vec![
            "pane".into(),
            "rename".into(),
            "w3Y:p1".into(),
            "{text}".into(),
        ];
        picked.prompt = Some("New pane name".into());
        let mut app = app_with(vec![picked.clone()]);
        app.enter_prompt(picked, String::new());

        let lines = draw(&mut app, 36, 8);
        assert!(!lines.last().unwrap().contains("0/0"), "{lines:#?}");
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

    /// Herdr's stderr quoted whole (#144): many lines, more than the cap.
    fn long_error() -> String {
        (0..20)
            .map(|i| format!("herdr: error line {i}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_long_status_takes_five_rows_and_says_it_was_cut() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        app.status = Some(long_error());
        let lines = draw(&mut app, 36, 20);
        assert_eq!(
            lines[15..],
            [
                "herdr: error line 0",
                "herdr: error line 1",
                "herdr: error line 2",
                "herdr: error line 3",
                "herdr: error line 4...",
            ],
            "{lines:#?}"
        );
        assert!(!lines[14].contains("herdr"), "{lines:#?}");
    }

    /// Issues #149 and #154. The manifest's 60-column popup is 58 inside
    /// Herdr's border, and a narrow device clamps it to 51 (docs/design.md §5).
    #[test]
    fn every_startup_note_fits_whole_inside_the_popup() {
        let dir = std::env::temp_dir().join(format!("palette-ui-notes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(crate::settings::FILE_NAME),
            "icons = false\n[auto_name]\nbase_url = \"https://x\"\n",
        )
        .unwrap();
        let (_, unusable) = crate::settings::load(Some(&dir));
        let notes = [
            crate::outdated_note("0.1.0", "0.8.2"),
            crate::skipped_note(1),
            crate::retired_icon_note(),
            unusable.expect("an unusable value is reported"),
        ];
        for width in [58, 51] {
            let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
            for note in &notes {
                app.add_status(note.clone());
            }
            let lines = draw(&mut app, width, 20);
            let footer = usize::from(20 - MAX_STATUS_ROWS);
            let shown: String = lines[footer..].concat().split_whitespace().collect();
            for note in &notes {
                let note: String = note.split_whitespace().collect();
                assert!(shown.contains(&note), "{note} is not whole in {lines:#?}");
            }
            assert!(!shown.ends_with(CUT_MARKER), "{lines:#?}");
        }
    }

    /// A cut wide glyph would overprint the marker, so it is blanked instead.
    #[test]
    fn a_cut_marker_after_a_wide_glyph_at_the_edge_replaces_it_whole() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        // `ｶﾞ` is one column to unicode-width but two cells to ratatui.
        for glyph in ["あ", "ｶ\u{FF9E}"] {
            app.status = Some(glyph.repeat(100));
            let lines = draw(&mut app, 36, 20);
            // `draw` reads a wide glyph's second cell as a space.
            let row = format!("{} ...", format!("{glyph} ").repeat(16));
            assert_eq!(lines[19], row, "{glyph:?}: {lines:#?}");
        }
    }

    #[test]
    fn a_cut_marker_after_a_short_row_ending_in_a_two_cell_glyph_is_whole() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        app.status = Some("one\ntwo\nthree\nfour\nfive ｶ\u{FF9E}\nsix".to_string());
        let lines = draw(&mut app, 36, 20);
        assert_eq!(lines[19], "five ｶ\u{FF9E} ...", "{lines:#?}");
    }

    #[test]
    fn text_after_more_blank_lines_than_the_pane_has_rows_is_marked_cut() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        app.status = Some(format!("first{}last", "\n".repeat(25)));
        let lines = draw(&mut app, 36, 20);
        assert_eq!(lines[15..], ["first", "", "", "", "..."], "{lines:#?}");
    }

    #[test]
    fn trailing_blank_lines_take_no_rows_and_are_not_marked_cut() {
        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        app.status = Some(format!("only line{}", "\n".repeat(25)));
        let lines = draw(&mut app, 36, 20);
        assert_eq!(lines[19], "only line", "{lines:#?}");
        assert!(!lines[18].contains("only line"), "{lines:#?}");
    }

    /// The typing stage lists nothing, so it keeps no rows for candidates.
    #[test]
    fn a_long_status_while_typing_takes_three_rows_at_the_floor() {
        let mut picked = command("pane.rename", "Rename pane", None);
        picked.args = vec!["pane".into(), "rename".into(), "{text}".into()];
        picked.prompt = Some("New pane name".into());
        let mut app = app_with(vec![picked.clone()]);
        app.enter_prompt(picked, String::new());
        app.status = Some(long_error());
        let lines = draw(&mut app, 36, MIN_ROWS);
        assert_eq!(
            lines[3..],
            [
                "herdr: error line 0",
                "herdr: error line 1",
                "herdr: error line 2...",
            ],
            "{lines:#?}"
        );
    }

    #[test]
    fn a_four_row_status_while_typing_at_the_floor_is_marked_cut() {
        let mut picked = command("pane.rename", "Rename pane", None);
        picked.args = vec!["pane".into(), "rename".into(), "{text}".into()];
        picked.prompt = Some("New pane name".into());
        let mut app = app_with(vec![picked.clone()]);
        app.enter_prompt(picked, String::new());
        app.status = Some("line 0\nline 1\nline 2\nline 3".to_string());
        let lines = draw(&mut app, 36, MIN_ROWS);
        assert_eq!(lines[3..], ["line 0", "line 1", "line 2..."], "{lines:#?}");
    }

    /// The floor's two candidates outrank status rows, so there the status is
    /// one row — still marked as cut.
    #[test]
    fn a_long_status_leaves_the_floor_its_two_candidates() {
        let mut candidates = vec![Candidate::note("tab.rename", "no {text}")];
        candidates.extend((0..9).map(|i| {
            Candidate::from_command(command(&format!("c{i}"), &format!("Cmd {i}"), None))
        }));
        let mut app = App::new(candidates, Frecency::load(Path::new("/nonexistent")));
        app.status = Some(long_error());
        let lines = draw(&mut app, 36, MIN_ROWS);
        let listed = lines
            .iter()
            .filter(|l| l.contains("Cmd") || l.contains("skipped"))
            .count();
        assert_eq!(listed, 2, "Commands: {lines:#?}");
        assert_eq!(
            lines.last().unwrap(),
            "herdr: error line 0...",
            "{lines:#?}"
        );

        let picked = command("focus.tab", "Focus tab...", Some("tabs"));
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
        app.status = Some(long_error());
        let lines = draw(&mut app, 36, MIN_ROWS);
        let listed = lines.iter().filter(|l| l.contains("Target")).count();
        assert_eq!(listed, 2, "Targets: {lines:#?}");
        assert!(lines.last().unwrap().ends_with("..."), "{lines:#?}");
    }

    /// Issue #102: the skipped-entries note is 37 cells, so at the floor it would
    /// wrap and take a candidate's row.
    #[test]
    fn the_skipped_note_leaves_the_floor_its_two_candidates() {
        for width in [36, 30] {
            let mut candidates = vec![Candidate::note("tab.rename", "no {text}")];
            candidates.extend((0..9).map(|i| {
                Candidate::from_command(command(&format!("c{i}"), &format!("Cmd {i}"), None))
            }));
            let mut app = App::new(candidates, Frecency::load(Path::new("/nonexistent")));
            assert!(
                app.selected_note().is_some(),
                "the skip reason is not shown"
            );
            app.add_status(crate::skipped_note(1));
            let lines = draw(&mut app, width, MIN_ROWS);
            let listed = lines
                .iter()
                .filter(|l| l.contains("Cmd") || l.contains("skipped `"))
                .count();
            assert_eq!(listed, 2, "width {width}: {lines:#?}");
            assert_eq!(
                lines.last().unwrap(),
                "catalog: 1 skipped - search...",
                "width {width}: {lines:#?}"
            );
            assert!(!lines[4].contains("catalog"), "width {width}: {lines:#?}");
        }
    }

    /// Herdr can hand the plugin a region narrower than §5's floor. The counts
    /// are what has to survive there, so the version is dropped whole rather
    /// than either half being truncated.
    #[test]
    fn a_pane_too_narrow_for_both_drops_the_version() {
        let counts = "1/1 ⋅ esc to close";
        let version_width = 1 + env!("CARGO_PKG_VERSION").chars().count();
        let one_column_short = counts.chars().count() + version_width;

        let mut app = app_with(vec![command("split.right", "Split pane: right", None)]);
        let lines = draw(&mut app, one_column_short as u16, 8);
        assert_eq!(lines.last().unwrap(), counts, "{lines:#?}");
    }

    /// Two candidates in every stage is what the floor is derived from. The
    /// Commands stage with a skip reason selected is the tallest; Targets pays
    /// for a header line. More candidates than can fit, so each count is the
    /// list's height.
    #[test]
    fn every_stage_shows_two_candidates_at_the_height_floor() {
        let mut candidates = vec![Candidate::note("tab.rename", "no {text}")];
        candidates.extend((0..9).map(|i| {
            Candidate::from_command(command(&format!("c{i}"), &format!("Cmd {i}"), None))
        }));
        let mut app = App::new(candidates, Frecency::load(Path::new("/nonexistent")));
        assert!(
            app.selected_note().is_some(),
            "the skip reason is not shown"
        );
        let lines = draw(&mut app, 36, MIN_ROWS);
        let listed = lines
            .iter()
            .filter(|l| l.contains("Cmd") || l.contains("skipped"))
            .count();
        assert_eq!(listed, 2, "Commands: {lines:#?}");

        let picked = command("focus.tab", "Focus tab...", Some("tabs"));
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
        let lines = draw(&mut app, 36, MIN_ROWS);
        let listed = lines.iter().filter(|l| l.contains("Target")).count();
        assert!(listed >= 2, "Targets: {lines:#?}");
    }

    #[test]
    fn a_row_under_the_floor_replaces_the_list_with_what_it_needs() {
        let mut app = app_with(vec![command("tab.create", "New tab", None)]);
        let lines = draw(&mut app, 36, MIN_ROWS - 1);
        assert_eq!(lines[0], "Too short: needs 6 rows, has 5", "{lines:#?}");
        assert_eq!(lines[1], "Esc to close", "{lines:#?}");
        assert!(!lines.iter().any(|l| l.contains("New tab")), "{lines:#?}");
    }

    /// Width has no enforced floor (§5): a narrow pane truncates, it does not
    /// refuse.
    #[test]
    fn a_pane_narrower_than_the_advisory_width_still_draws_the_list() {
        let mut app = app_with(vec![command("tab.create", "New tab", None)]);
        let lines = draw(&mut app, 30, MIN_ROWS);
        assert!(lines.iter().any(|l| l.contains("New tab")), "{lines:#?}");
        assert!(!lines.iter().any(|l| l.contains("Too short")), "{lines:#?}");
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
    use crate::herdr::PluginAction;
    use crate::listing::Target;
    use crossterm::event::{KeyEvent, KeyEventState};
    use ratatui::backend::TestBackend;

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
        assert_eq!(input.trim_end(), "> split⎸");
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
