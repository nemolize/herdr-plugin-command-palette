//! The `Auto generate` row under a workspace rename's input: whether it holds
//! focus, and whether a name is being generated or the last attempt failed.
use crate::glyph::SEPARATOR;

const LABEL: &str = "\u{2728} Auto generate";
const GENERATING: &str = "\u{2728} Generating...";
/// Shorter than `LABEL` so the reason beside it fits the popup's width.
const RETRY: &str = "\u{2728} Retry";

#[derive(Debug, PartialEq)]
enum State {
    Idle,
    Generating { id: u64 },
    Failed(String),
}

#[derive(Debug, PartialEq)]
pub struct GenerateRow {
    pub focused: bool,
    state: State,
}

impl Default for GenerateRow {
    fn default() -> Self {
        Self {
            focused: false,
            state: State::Idle,
        }
    }
}

impl GenerateRow {
    pub fn label(&self) -> String {
        match &self.state {
            State::Idle => LABEL.to_string(),
            State::Generating { .. } => GENERATING.to_string(),
            State::Failed(why) => format!("{RETRY}{SEPARATOR}{why}"),
        }
    }

    pub fn is_generating(&self) -> bool {
        matches!(self.state, State::Generating { .. })
    }

    pub fn start(&mut self, id: u64) -> bool {
        if self.is_generating() {
            return false;
        }
        self.state = State::Generating { id };
        true
    }

    pub fn cancel(&mut self) -> bool {
        if !self.is_generating() {
            return false;
        }
        self.state = State::Idle;
        true
    }

    /// Applies the answer to request `id`, returning the name to put in the
    /// input. An answer to any other request — one cancelled, or replaced — is
    /// dropped and changes nothing.
    pub fn finish(&mut self, id: u64, answer: Result<String, String>) -> Option<String> {
        if self.state != (State::Generating { id }) {
            return None;
        }
        match answer {
            Ok(name) => {
                self.state = State::Idle;
                self.focused = false;
                Some(name)
            }
            Err(why) => {
                // The reason comes from a server and is drawn on one row, so a
                // newline in it must not break the row.
                let why = why.chars().map(|c| if c.is_control() { ' ' } else { c });
                self.state = State::Failed(why.collect::<String>().trim().to_string());
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_row_offers_to_generate() {
        assert_eq!(GenerateRow::default().label(), "\u{2728} Auto generate");
    }

    #[test]
    fn a_started_row_says_it_is_generating_and_takes_no_second_request() {
        let mut row = GenerateRow::default();
        assert!(row.start(1));
        assert_eq!(row.label(), "\u{2728} Generating...");
        assert!(!row.start(2), "one request at a time");
        assert!(row.is_generating());
    }

    #[test]
    fn a_success_hands_over_the_name_and_returns_focus_to_the_input() {
        let mut row = GenerateRow {
            focused: true,
            ..GenerateRow::default()
        };
        row.start(7);
        assert_eq!(row.finish(7, Ok("palette".into())), Some("palette".into()));
        assert!(!row.focused);
        assert_eq!(row.label(), "\u{2728} Auto generate");
    }

    #[test]
    fn a_failure_shows_its_reason_and_can_be_retried() {
        let mut row = GenerateRow {
            focused: true,
            ..GenerateRow::default()
        };
        row.start(1);
        assert_eq!(row.finish(1, Err("timed out\nafter 60s".into())), None);
        assert_eq!(row.label(), "\u{2728} Retry \u{22C5} timed out after 60s");
        assert!(row.focused, "still on the row, to retry");
        assert!(row.start(2), "retry");
    }

    #[test]
    fn a_cancelled_requests_late_answer_is_dropped() {
        let mut row = GenerateRow::default();
        row.start(1);
        assert!(row.cancel());
        assert!(!row.cancel(), "nothing left to cancel");
        assert_eq!(row.finish(1, Ok("late".into())), None);
        assert_eq!(row.finish(1, Err("late".into())), None);
        assert_eq!(row.label(), "\u{2728} Auto generate");

        row.start(2);
        assert_eq!(row.finish(1, Ok("stale".into())), None, "not this request");
        assert!(row.is_generating());
    }
}
