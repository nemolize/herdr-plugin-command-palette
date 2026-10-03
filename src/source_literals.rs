//! The string and char literals a source file ships: its tokens outside test
//! code and doc comments.

use proc_macro2::{Delimiter, LexError, TokenStream, TokenTree};

#[derive(Debug, PartialEq)]
pub struct Literal {
    pub line: usize,
    /// Escapes other than `\u{..}` are kept as written; they are all ASCII.
    pub text: String,
}

/// Every string and char literal in `src`, except those in doc comments and in
/// the item, field, arm or parameter a `#[cfg(test)]` attribute marks.
pub fn shipped_literals(src: &str) -> Result<Vec<Literal>, LexError> {
    let mut out = Vec::new();
    collect(src.parse()?, &mut out);
    Ok(out)
}

fn collect(tokens: TokenStream, out: &mut Vec<Literal>) {
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        match token {
            TokenTree::Punct(p) if p.as_char() == '#' => {
                let inner = tokens
                    .next_if(|t| matches!(t, TokenTree::Punct(p) if p.as_char() == '!'))
                    .is_some();
                let Some(TokenTree::Group(attr)) = tokens.next_if(
                    |t| matches!(t, TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket),
                ) else {
                    continue;
                };
                if !is_cfg_test(&attr.stream()) {
                    if !is_doc(&attr.stream()) {
                        collect(attr.stream(), out);
                    }
                    continue;
                }
                if inner {
                    return;
                }
                // The marked item ends at its body, a `;`, or a `,` between fields,
                // arms or parameters.
                for t in tokens.by_ref() {
                    match t {
                        TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => break,
                        TokenTree::Punct(p) if matches!(p.as_char(), ';' | ',') => break,
                        _ => {}
                    }
                }
            }
            TokenTree::Group(g) => collect(g.stream(), out),
            TokenTree::Literal(lit) => {
                if let Some(text) = contents(&lit.to_string()) {
                    out.push(Literal {
                        line: lit.span().start().line,
                        text,
                    });
                }
            }
            _ => {}
        }
    }
}

fn is_cfg_test(attr: &TokenStream) -> bool {
    attr.to_string().replace(' ', "") == "cfg(test)"
}

fn is_doc(attr: &TokenStream) -> bool {
    matches!(attr.clone().into_iter().next(), Some(TokenTree::Ident(i)) if i == "doc")
}

/// The contents of a string or char literal as written; None for a number.
fn contents(token: &str) -> Option<String> {
    let body = token.trim_start_matches(['b', 'c']);
    if let Some(raw) = body.strip_prefix('r') {
        let hashes = raw.len() - raw.trim_start_matches('#').len();
        return Some(raw[hashes + 1..raw.len() - hashes - 1].to_string());
    }
    let quote = body.chars().next().filter(|q| matches!(q, '"' | '\''))?;
    Some(decode(&body[1..body.rfind(quote).unwrap()]))
}

fn decode(escaped: &str) -> String {
    let mut text = String::new();
    let mut chars = escaped.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('u') => {
                let hex: String = chars.by_ref().take_while(|&h| h != '}').collect();
                let digits = hex.trim_start_matches('{').replace('_', "");
                let decoded = u32::from_str_radix(&digits, 16)
                    .ok()
                    .and_then(char::from_u32);
                text.push(decoded.unwrap_or_else(|| panic!("bad escape \\u{hex}}}")));
            }
            other => text.extend(std::iter::once('\\').chain(other)),
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::{shipped_literals, Literal};
    use crate::glyph::same_width_in_every_locale;

    fn texts(src: &str) -> Vec<String> {
        shipped_literals(src)
            .unwrap()
            .into_iter()
            .map(|l| l.text)
            .collect()
    }

    /// Runtime values inside a note are not covered: only what the source spells.
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
            let literals = shipped_literals(&src)
                .unwrap_or_else(|e| panic!("{} does not tokenize: {e}", path.display()));
            for Literal { line, text } in literals {
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
    fn literals_in_code_are_read_and_comments_and_docs_are_not() {
        let src = "/// \"doc\"\nlet a = \"x\"; // \"in a comment\"\n/* \"block\" */ let b = 'y';";
        assert_eq!(texts(src), ["x", "y"]);
    }

    #[test]
    fn a_test_item_is_skipped_through_its_closing_brace_or_semicolon() {
        let src = "fn a() { \"kept\"; }\n\
                   #[cfg(test)]\nconst X: &str = \"dropped too\";\n\
                   #[cfg(test)]\nmod tests { fn t() { \"dropped\"; } fn u() {} \"still dropped\"; }\n\
                   fn b() { \"kept too\"; }";
        assert_eq!(texts(src), ["kept", "kept too"]);
    }

    #[test]
    fn a_test_field_arm_or_parameter_is_skipped_through_its_comma() {
        let src = "struct S { #[cfg(test)] a: u8, b: u8 }\n\
                   fn f(#[cfg(test)] x: u8, y: u8) { \"param\"; }\n\
                   fn g(n: u8) { match n { #[cfg(test)] 0 => \"arm\", _ => \"other\" } }\n\
                   fn h() { \"after\"; }";
        assert_eq!(texts(src), ["param", "other", "after"]);
    }

    #[test]
    fn a_unicode_escape_is_decoded_with_its_underscores() {
        let src = "fn f<'a>(s: &'a str) { \"\\u{2014}\"; '\\''; '\\u{25_A1}'; \"\\\\u{2014}\"; }";
        assert_eq!(texts(src), ["\u{2014}", "\\'", "\u{25A1}", "\\\\u{2014}"]);
    }

    #[test]
    fn a_raw_string_keeps_its_contents_as_written() {
        let src = "let a = r#\"say \"hi\"\"#; let b = br\"\\\"; let c = \"//\";";
        assert_eq!(texts(src), ["say \"hi\"", "\\", "//"]);
    }

    #[test]
    fn a_literal_reports_the_line_it_starts_on() {
        let src = "\n\"one\nline two\"\n\"a\\u{A}b\"\n\"four\"";
        let lines: Vec<usize> = shipped_literals(src)
            .unwrap()
            .iter()
            .map(|l| l.line)
            .collect();
        assert_eq!(lines, [2, 4, 5]);
    }

    #[test]
    fn an_inner_test_attribute_skips_the_rest_of_its_module() {
        let src = "fn a() { \"kept\"; }\n\
                   mod m { #![cfg(test)] fn t() { \"dropped\"; } \"dropped too\"; }\n\
                   fn b() { \"kept too\"; }";
        assert_eq!(texts(src), ["kept", "kept too"]);
    }

    #[test]
    fn an_attribute_argument_is_read_but_a_doc_comment_is_not() {
        let src = "/// \"doc\"\n#[error(\"in attr\")]\nstruct E;";
        assert_eq!(texts(src), ["in attr"]);
    }

    #[test]
    fn source_that_does_not_tokenize_is_an_error() {
        assert!(shipped_literals("fn f() { \"unterminated }").is_err());
    }

    #[test]
    fn a_hash_that_opens_no_attribute_keeps_the_token_after_it() {
        let src = "macro_rules! m { () => { # \"after hash\" }; }";
        assert_eq!(texts(src), ["after hash"]);
    }
}
