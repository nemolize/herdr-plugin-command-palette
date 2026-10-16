//! Rules for the argv a picked entry runs with: filling a placeholder, and
//! refusing a typed name the command would read as one of its own flags.

/// Whether `text` would reach the command as one of its own flags.
///
/// Measured on 0.9.0: `pane rename --clear` deletes the name and exits 0, while
/// every other `-`-leading name is stored verbatim by all three commands.
///
/// Matched on the subcommand pair rather than the first argument, so a leading
/// global flag (`--session x pane rename …`) cannot walk past the guard and a
/// sibling subcommand (`pane send-text`) is not caught by it.
pub fn eaten_as_a_flag(args: &[String], text: &str) -> bool {
    if text != "--clear" {
        return false;
    }
    // `--session` and `--remote` consume the token after them, which would
    // otherwise read as the subcommand and walk the pair out of alignment.
    const GLOBAL_FLAGS_TAKING_A_VALUE: [&str; 3] =
        ["--session", "--remote", "--remote-keybindings"];

    let mut rest = args.iter().map(String::as_str);
    let mut verbs = Vec::new();
    while let Some(a) = rest.next() {
        if GLOBAL_FLAGS_TAKING_A_VALUE.contains(&a) {
            rest.next();
        } else if !a.starts_with('-') {
            verbs.push(a);
            if verbs.len() == 2 {
                break;
            }
        }
    }
    matches!(verbs.as_slice(), ["pane", "rename"])
}

/// Fills one placeholder — `{}` with a chosen id, `{text}` with a typed name.
///
/// The value lands in exactly one argv element and the surrounding argv is
/// untouched, so a value that looks like a placeholder cannot rewrite the
/// command around it, and a name with spaces cannot split into several
/// arguments to a variadic `LABEL...`.
pub fn substitute(args: &[String], placeholder: &str, value: &str) -> Vec<String> {
    args.iter()
        .map(|a| {
            if a == placeholder {
                value.to_string()
            } else {
                a.clone()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_only_the_placeholder() {
        let args = ["tab".to_string(), "focus".to_string(), "{}".to_string()];
        assert_eq!(
            substitute(&args, "{}", "w3Y:t1"),
            vec!["tab", "focus", "w3Y:t1"]
        );
    }

    #[test]
    fn substitution_leaves_a_placeholder_shaped_id_inert() {
        let args = ["tab".to_string(), "focus".to_string(), "{}".to_string()];
        // The id lands in the placeholder slot; it does not rewrite its neighbours.
        assert_eq!(substitute(&args, "{}", "{}"), vec!["tab", "focus", "{}"]);
    }

    /// The guard reads the subcommand pair, so neither a global flag in front of
    /// it nor a sibling `pane` subcommand shifts what it matches.
    #[test]
    fn the_flag_guard_matches_the_subcommand_not_the_first_argument() {
        let argv =
            |parts: &[&str]| -> Vec<String> { parts.iter().map(|s| s.to_string()).collect() };

        assert!(eaten_as_a_flag(&argv(&["pane", "rename", "p1"]), "--clear"));
        assert!(
            eaten_as_a_flag(
                &argv(&["--session", "work", "pane", "rename", "p1"]),
                "--clear"
            ),
            "a value-taking global flag must not shift the pair"
        );
        assert!(
            !eaten_as_a_flag(&argv(&["pane", "send-text", "p1"]), "--clear"),
            "a sibling subcommand has no such flag"
        );
        assert!(!eaten_as_a_flag(&argv(&["tab", "rename", "t1"]), "--clear"));
        assert!(
            !eaten_as_a_flag(&argv(&["pane", "rename", "p1"]), "-x"),
            "only that one literal is eaten"
        );
    }
}
