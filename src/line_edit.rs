//! A one-line text buffer with a cursor, and the readline keys that edit it —
//! shared by the query and the typed name (docs/design.md §4).
//!
//! The cursor moves and deletes by grapheme cluster, so it never stands inside
//! a glyph: the renderer splits the text at it, and a split cluster draws as a
//! different glyph.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Default)]
pub struct LineEdit {
    text: String,
    /// A byte offset into `text`, always on a cluster boundary.
    cursor: usize,
}

/// What a key does to the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Insert(char),
    DeleteBack,
    DeleteForward,
    Left,
    Right,
    Home,
    End,
    WordLeft,
    WordRight,
    DeleteWordBack,
    DeleteWordForward,
    KillToStart,
    KillToEnd,
}

impl Edit {
    /// The readline meaning of `key`, or None for a key that is not an edit.
    ///
    /// `Ctrl-H` is Backspace on terminals that send `^H` for it. SHIFT is what
    /// produces capitals, so it is the one modifier a typed character may carry.
    pub fn from_key(key: KeyEvent) -> Option<Self> {
        let ctrl = key.modifiers == KeyModifiers::CONTROL;
        let alt = key.modifiers == KeyModifiers::ALT;
        Some(match key.code {
            KeyCode::Backspace if alt => Edit::DeleteWordBack,
            KeyCode::Backspace => Edit::DeleteBack,
            KeyCode::Delete => Edit::DeleteForward,
            KeyCode::Left => Edit::Left,
            KeyCode::Right => Edit::Right,
            KeyCode::Home => Edit::Home,
            KeyCode::End => Edit::End,
            KeyCode::Char(c) if ctrl => match c {
                'a' => Edit::Home,
                'e' => Edit::End,
                'b' => Edit::Left,
                'f' => Edit::Right,
                'h' => Edit::DeleteBack,
                'd' => Edit::DeleteForward,
                'w' => Edit::DeleteWordBack,
                'u' => Edit::KillToStart,
                'k' => Edit::KillToEnd,
                _ => return None,
            },
            KeyCode::Char(c) if alt => match c {
                'b' => Edit::WordLeft,
                'f' => Edit::WordRight,
                'd' => Edit::DeleteWordForward,
                _ => return None,
            },
            KeyCode::Char(c) if (key.modifiers - KeyModifiers::SHIFT).is_empty() => Edit::Insert(c),
            _ => return None,
        })
    }
}

impl LineEdit {
    /// `text` with the cursor at its end, where typing continues it.
    pub fn new(text: String) -> Self {
        let cursor = text.len();
        Self { text, cursor }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text either side of the cursor.
    pub fn split(&self) -> (&str, &str) {
        self.text.split_at(self.cursor)
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Whether the text changed — a deletion at the edge it deletes towards
    /// changes nothing, like a cursor movement.
    pub fn apply(&mut self, edit: Edit) -> bool {
        let before = self.text.len();
        match edit {
            Edit::Insert(c) => {
                self.text.insert(self.cursor, c);
                // A combining mark joins the cluster before it, so the cursor
                // re-settles on a boundary rather than inside that cluster.
                self.cursor = self.boundary_at_or_after(self.cursor + c.len_utf8());
            }
            Edit::DeleteBack => self.delete_to(self.prev_boundary(self.cursor)),
            Edit::DeleteForward => self.delete_to(self.next_boundary(self.cursor)),
            Edit::Left => self.cursor = self.prev_boundary(self.cursor),
            Edit::Right => self.cursor = self.next_boundary(self.cursor),
            Edit::Home => self.cursor = 0,
            Edit::End => self.cursor = self.text.len(),
            Edit::WordLeft => self.cursor = self.word_start(),
            Edit::WordRight => self.cursor = self.word_end(),
            Edit::DeleteWordBack => self.delete_to(self.word_start()),
            Edit::DeleteWordForward => self.delete_to(self.word_end()),
            Edit::KillToStart => self.delete_to(0),
            Edit::KillToEnd => self.delete_to(self.text.len()),
        }
        self.text.len() != before
    }

    /// Removes the text between the cursor and `to`, leaving the cursor where
    /// the removed text began.
    fn delete_to(&mut self, to: usize) {
        let (start, end) = (self.cursor.min(to), self.cursor.max(to));
        self.text.replace_range(start..end, "");
        // The text either side can join into one cluster once the gap closes.
        self.cursor = self.boundary_at_or_before(start);
    }

    fn prev_boundary(&self, at: usize) -> usize {
        self.text[..at]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self, at: usize) -> usize {
        self.text[at..]
            .graphemes(true)
            .next()
            .map_or(at, |g| at + g.len())
    }

    /// Where the word before the cursor starts. A word is a run of
    /// non-whitespace, as `Ctrl-W` reads it, so `pane:right` is one word.
    fn word_start(&self) -> usize {
        let before = self.text[..self.cursor].trim_end();
        self.boundary_at_or_before(before.len() - word_len(before.chars().rev()))
    }

    /// Where the word after the cursor ends.
    fn word_end(&self) -> usize {
        let after = &self.text[self.cursor..];
        let word = after.trim_start();
        self.boundary_at_or_after(self.cursor + (after.len() - word.len()) + word_len(word.chars()))
    }

    // Word bounds are measured in chars, and a space can share a cluster with a
    // mark either side of it: widening a word outward keeps every word key moving.
    fn boundary_at_or_before(&self, at: usize) -> usize {
        self.boundaries()
            .take_while(|&i| i <= at)
            .last()
            .unwrap_or(0)
    }

    fn boundary_at_or_after(&self, at: usize) -> usize {
        self.boundaries()
            .find(|&i| i >= at)
            .unwrap_or(self.text.len())
    }

    /// Every cluster boundary, the end of the text included.
    fn boundaries(&self) -> impl Iterator<Item = usize> + '_ {
        self.text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .chain(std::iter::once(self.text.len()))
    }
}

/// Bytes in the run of non-whitespace `chars` opens with.
fn word_len(chars: impl Iterator<Item = char>) -> usize {
    chars
        .take_while(|c| !c.is_whitespace())
        .map(char::len_utf8)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::{Edit, LineEdit};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// The buffer drawn with `|` at the cursor — literal expectations, so a
    /// wrong cursor reads as a wrong string rather than a wrong number.
    fn shown(e: &LineEdit) -> String {
        let (before, after) = e.split();
        format!("{before}|{after}")
    }

    fn after(start: &str, cursor_from_end: usize, edits: &[Edit]) -> String {
        let mut e = LineEdit::new(start.to_string());
        for _ in 0..cursor_from_end {
            e.apply(Edit::Left);
        }
        for &edit in edits {
            e.apply(edit);
        }
        shown(&e)
    }

    #[test]
    fn a_new_buffer_has_its_cursor_at_the_end() {
        assert_eq!(shown(&LineEdit::new("tab".into())), "tab|");
    }

    #[test]
    fn each_edit_lands_where_readline_puts_it() {
        use Edit::*;
        let cases: &[(&str, usize, &[Edit], &str)] = &[
            ("split", 0, &[Insert('s')], "splits|"),
            ("split", 2, &[Insert('x')], "splx|it"),
            ("split", 0, &[DeleteBack], "spli|"),
            ("split", 2, &[DeleteBack], "sp|it"),
            ("split", 5, &[DeleteBack], "|split"),
            ("split", 2, &[DeleteForward], "spl|t"),
            ("split", 0, &[DeleteForward], "split|"),
            ("split", 0, &[Home], "|split"),
            ("split", 3, &[End], "split|"),
            ("split", 5, &[Left], "|split"),
            ("split", 0, &[Right], "split|"),
            ("split pane", 2, &[KillToStart], "|ne"),
            ("split pane", 2, &[KillToEnd], "split pa|"),
            ("split pane", 0, &[KillToStart], "|"),
            ("split pane right", 0, &[DeleteWordBack], "split pane |"),
            ("split pane  ", 0, &[DeleteWordBack], "split |"),
            ("split pane", 2, &[DeleteWordBack], "split |ne"),
            ("split", 0, &[DeleteWordBack], "|"),
            ("pane:right x", 2, &[DeleteWordBack], "| x"),
            ("split pane", 0, &[WordLeft], "split |pane"),
            ("split pane", 0, &[WordLeft, WordLeft], "|split pane"),
            ("split  pane", 11, &[WordRight], "split|  pane"),
            ("split  pane", 6, &[WordRight], "split  pane|"),
            ("split pane right", 11, &[DeleteWordForward], "split| right"),
        ];
        for (start, left, edits, expected) in cases {
            assert_eq!(
                &after(start, *left, edits),
                expected,
                "{start:?}, {left} left, {edits:?}"
            );
        }
    }

    /// A cluster is one step, so the cursor never splits a glyph.
    #[test]
    fn the_cursor_steps_over_whole_clusters() {
        use Edit::*;
        let cases: &[(&str, usize, &[Edit], &str)] = &[
            ("あいう", 1, &[], "あい|う"),
            ("あいう", 1, &[DeleteBack], "あ|う"),
            ("e\u{301}x", 1, &[], "e\u{301}|x"),
            ("e\u{301}x", 1, &[DeleteBack], "|x"),
            ("\u{1F469}\u{200D}\u{1F4BB}a", 1, &[DeleteBack], "|a"),
            ("\u{1F1EF}\u{1F1F5}", 0, &[DeleteBack], "|"),
            ("ｶﾞ", 1, &[], "|ｶﾞ"),
            ("ab \u{301}cd", 0, &[WordLeft], "ab| \u{301}cd"),
            ("ab \u{301}cd", 0, &[WordLeft, WordLeft], "|ab \u{301}cd"),
            ("ab \u{301}cd", 0, &[DeleteWordBack], "ab|"),
            ("foo \u{301}", 0, &[WordLeft, WordLeft], "|foo \u{301}"),
            ("foo \u{301}", 0, &[DeleteWordBack, DeleteWordBack], "|"),
            ("a\t\u{301}", 2, &[DeleteForward, DeleteForward], "|"),
            ("\u{1F1E6}x\u{1F1E7}", 1, &[DeleteBack, DeleteForward], "|"),
            ("ab \u{301}cd", 5, &[WordRight], "ab| \u{301}cd"),
            ("x\u{600} y", 3, &[WordRight], "x\u{600} |y"),
            ("x\u{600} y", 3, &[DeleteWordForward], "|y"),
        ];
        for (start, left, edits, expected) in cases {
            assert_eq!(&after(start, *left, edits), expected, "{start:?}");
        }
    }

    /// The caller refilters and retracts a refusal only on a change, so a
    /// deletion with nothing to delete must not report one.
    #[test]
    fn only_an_edit_that_alters_the_text_reports_a_change() {
        use Edit::*;
        let cases: &[(&str, usize, Edit, bool)] = &[
            ("ab", 0, DeleteForward, false),
            ("ab", 0, KillToEnd, false),
            ("ab", 2, DeleteBack, false),
            ("ab", 2, DeleteWordBack, false),
            ("ab", 2, KillToStart, false),
            ("", 0, DeleteBack, false),
            ("ab", 1, Left, false),
            ("ab", 0, DeleteBack, true),
            ("ab", 1, DeleteForward, true),
            ("ab", 0, Insert('c'), true),
        ];
        for (start, left, edit, changed) in cases {
            let mut e = LineEdit::new(start.to_string());
            for _ in 0..*left {
                e.apply(Edit::Left);
            }
            assert_eq!(e.apply(*edit), *changed, "{start:?}, {left} left, {edit:?}");
        }
    }

    /// Typed before a combining mark, a letter takes the mark; the cursor
    /// follows to after the joined cluster rather than stopping inside it.
    #[test]
    fn a_letter_typed_before_a_combining_mark_leaves_the_cursor_on_a_boundary() {
        let mut e = LineEdit::new("\u{301}".into());
        e.apply(Edit::Home);
        e.apply(Edit::Insert('e'));
        assert_eq!(shown(&e), "e\u{301}|");
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn readline_keys_map_to_their_edits() {
        let ctrl = KeyModifiers::CONTROL;
        let alt = KeyModifiers::ALT;
        let none = KeyModifiers::NONE;
        let cases = [
            (key(KeyCode::Char('a'), ctrl), Some(Edit::Home)),
            (key(KeyCode::Char('e'), ctrl), Some(Edit::End)),
            (key(KeyCode::Char('b'), ctrl), Some(Edit::Left)),
            (key(KeyCode::Char('f'), ctrl), Some(Edit::Right)),
            (key(KeyCode::Char('h'), ctrl), Some(Edit::DeleteBack)),
            (key(KeyCode::Char('d'), ctrl), Some(Edit::DeleteForward)),
            (key(KeyCode::Char('w'), ctrl), Some(Edit::DeleteWordBack)),
            (key(KeyCode::Char('u'), ctrl), Some(Edit::KillToStart)),
            (key(KeyCode::Char('k'), ctrl), Some(Edit::KillToEnd)),
            (key(KeyCode::Char('b'), alt), Some(Edit::WordLeft)),
            (key(KeyCode::Char('f'), alt), Some(Edit::WordRight)),
            (key(KeyCode::Char('d'), alt), Some(Edit::DeleteWordForward)),
            (key(KeyCode::Backspace, alt), Some(Edit::DeleteWordBack)),
            (key(KeyCode::Backspace, none), Some(Edit::DeleteBack)),
            (key(KeyCode::Delete, none), Some(Edit::DeleteForward)),
            (key(KeyCode::Left, none), Some(Edit::Left)),
            (key(KeyCode::Right, none), Some(Edit::Right)),
            (key(KeyCode::Home, none), Some(Edit::Home)),
            (key(KeyCode::End, none), Some(Edit::End)),
            (
                key(KeyCode::Char('A'), KeyModifiers::SHIFT),
                Some(Edit::Insert('A')),
            ),
            (key(KeyCode::Char('a'), none), Some(Edit::Insert('a'))),
            (key(KeyCode::Char('x'), ctrl), None),
            (key(KeyCode::Char('x'), alt), None),
            (key(KeyCode::Char('a'), ctrl | alt), None),
            (key(KeyCode::Up, none), None),
        ];
        for (k, expected) in cases {
            assert_eq!(Edit::from_key(k), expected, "{k:?}");
        }
    }
}
