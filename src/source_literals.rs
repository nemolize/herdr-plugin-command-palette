//! The string and char literals a source file ships, read without a Rust
//! parser: enough to guard what text can reach the screen.

/// `text` has its `\u{..}` escapes decoded, so an escaped character is checked
/// like a typed one.
#[derive(Debug, PartialEq)]
pub struct Literal {
    pub line: usize,
    pub text: String,
}

/// Every string and char literal in `src`, except those inside comments and
/// inside the item a `#[cfg(test)]` attribute marks.
pub fn shipped_literals(src: &str) -> Vec<Literal> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut line = 1;
    let mut i = 0;
    let mut nesting = 0usize;
    let mut code_without_comments = String::new();
    let mut test_item_nesting: Option<usize> = None;

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '/' && next == Some('/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && next == Some('*') {
            let mut depth = 0;
            while i < chars.len() {
                match (chars[i], chars.get(i + 1).copied()) {
                    ('/', Some('*')) => (depth, i) = (depth + 1, i + 2),
                    ('*', Some('/')) => (depth, i) = (depth - 1, i + 2),
                    (ch, _) => {
                        line += usize::from(ch == '\n');
                        i += 1;
                    }
                }
                if depth == 0 {
                    break;
                }
            }
            continue;
        }
        let prev_ident = i > 0 && (chars[i - 1].is_alphanumeric() || chars[i - 1] == '_');
        let raw_hashes = (c == 'r' && !prev_ident)
            .then(|| chars[i + 1..].iter().take_while(|&&h| h == '#').count())
            .filter(|&n| chars.get(i + 1 + n) == Some(&'"'));
        if c == '"' || raw_hashes.is_some() {
            let start = line;
            let (text, end) = match raw_hashes {
                Some(n) => raw_string(&chars, i + 2 + n, n),
                None => cooked(&chars, i + 1, '"'),
            };
            line += text.matches('\n').count();
            if test_item_nesting.is_none() {
                out.push(Literal { line: start, text });
            }
            code_without_comments.push('"');
            i = end;
            continue;
        }
        if c == '\'' && is_char_literal(&chars, i) {
            let (text, end) = cooked(&chars, i + 1, '\'');
            if test_item_nesting.is_none() {
                out.push(Literal { line, text });
            }
            code_without_comments.push('\'');
            i = end;
            continue;
        }

        line += usize::from(c == '\n');
        code_without_comments.push(c);
        match c {
            '(' | '[' | '{' => nesting += 1,
            ')' | ']' | '}' => {
                nesting -= 1;
                if c == '}' && test_item_nesting == Some(nesting) {
                    test_item_nesting = None;
                }
            }
            ';' if test_item_nesting == Some(nesting) => test_item_nesting = None,
            _ => {}
        }
        // After the `]` above, so the item is measured at the attribute's own nesting.
        if code_without_comments.ends_with("#[cfg(test)]") {
            test_item_nesting.get_or_insert(nesting);
        }
        i += 1;
    }
    out
}

/// A `'` opens a char literal unless it is a lifetime or label (`'a`, `'_`).
fn is_char_literal(chars: &[char], i: usize) -> bool {
    matches!(
        (chars.get(i + 1), chars.get(i + 2)),
        (Some('\\'), _) | (Some(_), Some('\''))
    )
}

/// Reads up to the closing `quote`, decoding `\u{..}`.
fn cooked(chars: &[char], from: usize, quote: char) -> (String, usize) {
    let mut text = String::new();
    let mut i = from;
    while i < chars.len() && chars[i] != quote {
        if chars[i] == '\\' && chars.get(i + 1) == Some(&'u') {
            let close = (i..chars.len()).find(|&j| chars[j] == '}').unwrap();
            let hex: String = chars[i + 3..close].iter().collect();
            text.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            i = close + 1;
            continue;
        }
        if chars[i] == '\\' {
            text.push(chars[i]);
            i += 1;
        }
        text.extend(chars.get(i));
        i += 1;
    }
    (text, i + 1)
}

fn raw_string(chars: &[char], from: usize, hashes: usize) -> (String, usize) {
    let closes = |j: usize| chars[j] == '"' && (1..=hashes).all(|k| chars.get(j + k) == Some(&'#'));
    let end = (from..chars.len())
        .find(|&j| closes(j))
        .unwrap_or(chars.len());
    (chars[from..end].iter().collect(), end + 1 + hashes)
}

#[cfg(test)]
mod tests {
    use super::{shipped_literals, Literal};
    use crate::glyph::same_width_in_every_locale;

    fn texts(src: &str) -> Vec<String> {
        shipped_literals(src).into_iter().map(|l| l.text).collect()
    }

    /// Any text the palette draws is built from these literals plus runtime
    /// values, so an Ambiguous one would shift its row in a CJK locale.
    #[test]
    fn no_shipped_literal_widens_in_a_cjk_locale() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut scanned = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let src = std::fs::read_to_string(&path).unwrap();
            for Literal { line, text } in shipped_literals(&src) {
                assert!(
                    same_width_in_every_locale(&text),
                    "{}:{line}: {text:?}",
                    path.display()
                );
            }
            scanned += 1;
        }
        assert!(scanned > 1, "no source found under {}", dir.display());
    }

    #[test]
    fn literals_in_code_are_read_and_comments_are_not() {
        let src = "let a = \"x\"; // \"in a comment\"\n/* \"block\" */ let b = 'y';";
        assert_eq!(texts(src), ["x", "y"]);
    }

    #[test]
    fn a_test_item_nesting_is_skipped_through_its_closing_brace_or_semicolon() {
        let src = "fn a() { \"kept\"; }\n\
                   #[cfg(test)]\nmod tests { fn t() { \"dropped\"; } fn u() {} \"still dropped\"; }\n\
                   #[cfg(test)]\nuse x::{y};\n\
                   fn b() { \"kept too\"; }";
        assert_eq!(texts(src), ["kept", "kept too"]);
    }

    #[test]
    fn an_escaped_character_is_decoded_and_a_lifetime_is_not_a_literal() {
        let src = "fn f<'a>(s: &'a str) { \"\\u{2014}\"; '\\''; '\\u{25A1}'; }";
        assert_eq!(texts(src), ["\u{2014}", "\\'", "\u{25A1}"]);
    }

    #[test]
    fn a_raw_string_ends_at_its_own_hashes() {
        let src = "let a = r#\"say \"hi\"\"#; let b = r\"\\d\"; let c = \"//\";";
        assert_eq!(texts(src), ["say \"hi\"", "\\d", "//"]);
    }

    #[test]
    fn a_literal_reports_the_line_it_starts_on() {
        let src = "\n\"one\nline two\"\n\"three\"";
        let lines: Vec<usize> = shipped_literals(src).iter().map(|l| l.line).collect();
        assert_eq!(lines, [2, 4]);
    }
}
