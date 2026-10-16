//! Pane entrypoint (docs/design.md §3). All rendering, input, and dispatch live
//! here; the action hop only opens the pane.
mod app;
mod argv;
mod brief;
mod candidates;
mod catalog;
mod context;
mod frecency;
mod fuzzy;
mod generate_row;
mod glyph;
mod herdr;
mod input;
mod keys;
mod line_edit;
mod listing;
mod ollama;
mod selection;
mod settings;
#[cfg(test)]
mod source_literals;
mod status;
mod ui;
mod width;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{channel, Sender};
use std::time::{Duration, Instant};

use app::{App, Candidate, Outcome, Step};
use candidates::assemble;
use context::Context;
use frecency::Frecency;
use herdr::Herdr;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("command palette: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).map(PathBuf::from)
}

/// The current name of what a rename entry acts on, to seed its input with.
///
/// The argv is the source because the context ids are already substituted into
/// it by the time an entry becomes a candidate — so `["tab", "rename", "<id>",
/// …]` carries both the listing to search and the id to find.
fn seed_for(herdr: &Herdr, command: &catalog::Command) -> String {
    let (Some(subject), Some(verb), Some(id)) = (
        command.args.first(),
        command.args.get(1),
        command.args.get(2),
    ) else {
        return String::new();
    };
    if verb != "rename" || id.starts_with('{') {
        return String::new();
    }
    herdr.current_label(&format!("{subject} list"), id)
}

/// Fills `{repo}`, which unlike the context ids costs a herdr call.
fn fill_repo(herdr: &Herdr, context: &Context, args: &[String]) -> Result<Vec<String>, String> {
    if !args.iter().any(|a| a == "{repo}") {
        return Ok(args.to_vec());
    }
    let workspace = context
        .workspace_id
        .as_deref()
        .ok_or_else(|| "no workspace to find the repository of".to_string())?;
    let root = herdr.repo_root(workspace)?;
    Ok(args
        .iter()
        .map(|a| {
            if a == "{repo}" {
                root.clone()
            } else {
                a.clone()
            }
        })
        .collect())
}

fn targets_for(
    herdr: &Herdr,
    command: &catalog::Command,
    context: &Context,
) -> Result<Vec<listing::Target>, String> {
    let resolve = command.resolve.as_deref().unwrap_or_default();
    if resolve != "worktree list" {
        return herdr.targets(resolve);
    }
    let key = listing::WorktreeKey::for_args(&command.args)
        .ok_or_else(|| catalog::rejection(command).unwrap_or_default())?;
    let workspace = context
        .workspace_id
        .as_deref()
        .ok_or_else(|| "no workspace to list the repository of".to_string())?;
    herdr.worktrees(key, workspace)
}

fn run() -> Result<(), String> {
    let bin = std::env::var("HERDR_BIN_PATH")
        .map_err(|_| "HERDR_BIN_PATH is not set - this runs as a herdr plugin pane".to_string())?;
    let plugin_root =
        env_path("HERDR_PLUGIN_ROOT").ok_or_else(|| "HERDR_PLUGIN_ROOT is not set".to_string())?;
    let state_dir = env_path("HERDR_PLUGIN_STATE_DIR");
    let config_dir = env_path("HERDR_PLUGIN_CONFIG_DIR");

    let plugin_id = std::env::var("HERDR_PLUGIN_ID").ok();
    let herdr = Herdr::new(bin);

    let catalog_path = catalog::locate(&plugin_root, config_dir.as_deref());
    let catalog = catalog::load(&catalog_path)?;

    // What the user was looking at when the palette opened. The pane process
    // receives no ids of its own, so this JSON is the only route to them (§8).
    let context = Context::from_env();

    let checked_against = catalog.checked_against.clone();
    let sets_retired_icon = catalog.commands.iter().any(|c| c.retired_icon.is_some());

    // Defaulted rather than propagated because a palette of built-ins is still
    // a working palette (§4).
    let actions = herdr.plugin_actions().unwrap_or_default();
    // Both halves default away because a palette that cannot name the
    // shortcuts is still a working palette (§4).
    let bindings = keys::resolve(
        &herdr.default_config().unwrap_or_default(),
        keys::config_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .as_deref(),
    );

    let (mut candidates, rejected) = assemble(
        catalog.commands,
        actions,
        &context,
        plugin_id.as_deref(),
        herdr::current_platform(),
        &bindings,
    );

    if candidates.is_empty() {
        // The reasons are the whole diagnosis when nothing is left to offer, so
        // they go in the message rather than a list that never renders.
        let why = match rejected.is_empty() {
            true => String::new(),
            false => format!(
                " - skipped {}",
                rejected
                    .iter()
                    .map(|(id, why)| format!("{id}: {why}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        };
        return Err(format!(
            "no commands available (catalog: {}){why}",
            catalog_path.display()
        ));
    }

    candidates.extend(rejected.iter().map(|(id, why)| Candidate::note(id, why)));

    let frecency_path = state_dir.as_deref().map(frecency::path);
    let frecency = frecency_path
        .as_deref()
        .map(Frecency::load)
        .unwrap_or_default();

    let (settings, settings_problem) = settings::load(config_dir.as_deref());

    let mut app = App::new(candidates, frecency);

    // The catalog drifts when Herdr changes its CLI and nothing detects that
    // automatically (§4). A running Herdr older than the version the catalog was
    // checked against is the one case that IS detectable, so it is surfaced —
    // as a footer note rather than a refusal, because most entries still work.
    if let (Some(required), Some(actual)) = (checked_against.as_deref(), herdr.version()) {
        if catalog::is_older(&actual, required) == Some(true) {
            app.add_status(outdated_note(&actual, required));
        }
    }

    if !rejected.is_empty() {
        app.add_status(skipped_note(rejected.len()));
    }

    if sets_retired_icon {
        app.add_status(retired_icon_note());
    }

    if let Some(problem) = settings_problem {
        app.add_status(problem);
    }

    let mut screen = ui::Screen::enter()?;

    // Generation runs off the event loop so Esc can still cancel it.
    let (answers_tx, answers) = channel::<Answer>();

    // A step the loop produced itself, handled before the next keypress.
    let mut next: Option<Step> = None;
    loop {
        while let Ok((id, answer, elapsed)) = answers.try_recv() {
            app.generated(id, answer, elapsed);
        }
        let step = match next.take() {
            Some(step) => step,
            None => {
                let drawn_rows = screen.draw(&mut app)?;
                input::next_step(&mut app, drawn_rows)?
            }
        };
        match step {
            Step::Continue => {}
            Step::Cancel => return Ok(()),
            Step::NeedsRepo(mut command) => match fill_repo(&herdr, &context, &command.args) {
                Ok(args) => {
                    command.args = args;
                    next = Some(app.picked(command));
                }
                Err(e) => app.status = Some(format!("`{}` failed: {e}", command.id)),
            },
            Step::NeedsTargets(command) => {
                let resolve = command.resolve.clone().unwrap_or_default();
                match targets_for(&herdr, &command, &context) {
                    Ok(targets) if targets.is_empty() => {
                        app.status = Some(format!("nothing to pick from `{resolve}`"));
                    }
                    Ok(targets) => {
                        app.status = None;
                        app.enter_targets(command, targets);
                    }
                    Err(e) => app.status = Some(format!("{resolve}: {e}")),
                }
            }
            Step::NeedsPrompt(command) => {
                let seed = seed_for(&herdr, &command);
                app.enter_prompt(command, seed);
            }
            Step::Generate { id, subject } => generate(
                id,
                subject,
                &herdr,
                &context,
                &settings.auto_name,
                answers_tx.clone(),
            ),
            // The ranking is saved before the dispatch is attempted, so a
            // failure still leaves the ordering updated — the user did pick it.
            Step::Run(outcome) => {
                if let Some(path) = frecency_path.as_deref() {
                    let _ = app.frecency().save(path);
                }
                // Torn down first because the popup is a real pane while it is
                // up: an entry that moves focus would be racing its own UI.
                screen = match run_without_screen(screen, || dispatch(&herdr, outcome))? {
                    Dispatched::Ran => return Ok(()),
                    // Because a message printed after the popup closes is one
                    // nobody reads, the palette comes back carrying it instead.
                    Dispatched::Reopened(screen, e) => {
                        app.status = Some(e);
                        screen
                    }
                };
            }
        }
    }
}

/// A request's id, its outcome, and how long it took from the press.
type Answer = (u64, Result<String, String>, Duration);

/// A cancelled request still runs to its end, since a blocking call cannot be
/// aborted; the thread dies with the palette if it outlives it.
fn generate(
    id: u64,
    subject: brief::Subject,
    herdr: &Herdr,
    context: &Context,
    settings: &settings::AutoName,
    answers: Sender<Answer>,
) {
    // The palette's pane belongs to the workspace and tab it opened in, which a
    // catalog entry naming another one by id is not.
    let belongs = match &subject {
        brief::Subject::Workspace(w) => context.workspace_id.as_ref() == Some(w),
        brief::Subject::Tab(t) => context.tab_id.as_ref() == Some(t),
        brief::Subject::Pane(_) => false,
    };
    let pane = context.focused_pane_id.clone().filter(|_| belongs);
    let (herdr, settings) = (herdr.clone(), settings.clone());
    let started = Instant::now();
    std::thread::spawn(move || {
        let answer = brief::gather(&herdr, &subject, pane.as_deref())
            .and_then(|prompt| ollama::generate(&settings, &subject.system_prompt(), &prompt));
        let _ = answers.send((id, answer, started.elapsed()));
    });
}

enum Dispatched {
    Ran,
    /// The palette is back up carrying a failure, or what the command said.
    Reopened(ui::Screen, String),
}

/// Drops the screen, runs `f`, and re-enters only to carry a message back to
/// the user — a command with nothing to say is the palette's exit, so restoring
/// it there would flash the popup back up after the command it was closed for.
fn run_without_screen<F>(screen: ui::Screen, f: F) -> Result<Dispatched, String>
where
    F: FnOnce() -> Result<Option<String>, String>,
{
    drop(screen);
    let message = match f() {
        Ok(None) => return Ok(Dispatched::Ran),
        Ok(Some(said)) | Err(said) => said,
    };
    // A screen that will not reopen has nowhere to show the message, so it
    // leaves as this function's own error and `main` prints it.
    let screen = ui::Screen::enter().map_err(|_| message.clone())?;
    Ok(Dispatched::Reopened(screen, message))
}

/// Runs what the user picked. A failure names the command id, so a drifted
/// catalog entry reports itself the first time it is used instead of silently
/// doing nothing (§4).
fn dispatch(herdr: &Herdr, outcome: Outcome) -> Result<Option<String>, String> {
    let (id, args) = argv(outcome);
    herdr
        .dispatch(&args)
        .map_err(|e| format!("`{id}` failed: {e}"))
}

/// The id to name in a failure, and the argv to spawn. Split from `dispatch` so
/// the assembly is checkable without a herdr binary.
fn argv(outcome: Outcome) -> (String, Vec<String>) {
    match outcome {
        Outcome::Command { id, args } => (id, args),
        Outcome::Action {
            id,
            plugin_id,
            action_id,
        } => (
            id,
            [
                "plugin", "action", "invoke", &action_id, "--plugin", &plugin_id,
            ]
            .map(str::to_owned)
            .to_vec(),
        ),
    }
}

fn outdated_note(actual: &str, required: &str) -> String {
    format!("herdr {actual} < catalog {required} - some entries may fail")
}

fn skipped_note(count: usize) -> String {
    format!("catalog: {count} skipped - search `skipped`")
}

fn retired_icon_note() -> String {
    "catalog: `icon` unsupported".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Workspace w1 with two panes in tab t1 and one in t2, w2 with one, and a
    /// `pane read` that answers only when asked for exactly 120 lines.
    fn stub_herdr(name: &str) -> Herdr {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("palette-gen-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("herdr");
        std::fs::write(
            &bin,
            r#"#!/bin/sh
case "$*" in
  "pane list --workspace w1")
    echo '{"result":{"panes":[{"terminal_title_stripped":"cargo test","foreground_cwd":"/nonexistent/a"},{"terminal_title_stripped":"vim notes","foreground_cwd":""}]}}' ;;
  "pane read w1:p1 --lines 120 --format text")
    echo 'last screen line' ;;
  "pane list --workspace w2")
    echo '{"result":{"panes":[{"terminal_title_stripped":"w2 pane","foreground_cwd":""}]}}' ;;
  "pane list")
    echo '{"result":{"panes":[{"tab_id":"w1:t1","terminal_title_stripped":"cargo test","foreground_cwd":"/nonexistent/a"},{"tab_id":"w1:t2","terminal_title_stripped":"logs","foreground_cwd":"/nonexistent/b"},{"tab_id":"w1:t1","terminal_title_stripped":"vim notes","foreground_cwd":""},{"tab_id":"w2:t1","terminal_title_stripped":"w2 pane","foreground_cwd":""}]}}' ;;
  "pane get w1:p3")
    echo '{"result":{"pane":{"tab_id":"w1:t2","terminal_title_stripped":"logs","foreground_cwd":"/nonexistent/b"},"type":"pane_info"}}' ;;
  "pane read w1:p3 --lines 120 --format text")
    echo 'tail -f server.log' ;;
  *) echo "unexpected: $*" >&2; exit 1 ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        Herdr::new(bin.to_string_lossy().into_owned())
    }

    fn fake_ollama(
        reply: &'static str,
        delay: std::time::Duration,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let _ = tx.send(String::from_utf8(body).unwrap());
                std::thread::sleep(delay);
                write!(
                    &stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{reply}",
                    reply.len()
                )
                .unwrap();
            }
        });
        (url, rx)
    }

    fn auto_name_at(base_url: String) -> settings::AutoName {
        settings::AutoName {
            base_url,
            timeout_secs: 5,
            ..settings::AutoName::default()
        }
    }

    #[test]
    fn a_generate_request_sends_one_prompt_with_every_pane_and_the_focused_output() {
        let (url, requests) = fake_ollama(
            r#"{"response":"\"tests\"\n"}"#,
            std::time::Duration::from_millis(300),
        );
        let (tx, answers) = std::sync::mpsc::channel();
        generate(
            9,
            workspace("w1"),
            &stub_herdr("ok"),
            &in_a_pane(),
            &auto_name_at(url),
            tx,
        );

        let timeout = std::time::Duration::from_secs(10);
        let (id, answer, elapsed) = answers.recv_timeout(timeout).unwrap();
        assert_eq!((id, answer), (9, Ok("tests".to_string())));
        assert!(
            elapsed >= std::time::Duration::from_millis(300),
            "the server's pause is counted: {elapsed:?}"
        );
        let body: serde_json::Value =
            serde_json::from_str(&requests.recv_timeout(timeout).unwrap()).unwrap();
        assert_eq!(
            body["prompt"],
            "Panes in this workspace:\n\
             - title: cargo test; cwd: /nonexistent/a; git branch: none\n\
             - title: vim notes; cwd: none; git branch: none\n\
             \n\
             Last 120 lines of the focused pane:\n\
             last screen line\n"
        );
        assert_eq!(body["model"], "qwen3.5:4b");
        assert!(
            requests
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "exactly one request"
        );
    }

    #[test]
    fn a_herdr_failure_is_the_answer_and_nothing_is_sent() {
        let (url, requests) = fake_ollama("{}", std::time::Duration::ZERO);
        let (tx, answers) = std::sync::mpsc::channel();
        let context = Context {
            focused_pane_id: Some("w1:p9".into()),
            ..in_a_pane()
        };
        generate(
            1,
            workspace("w1"),
            &stub_herdr("fail"),
            &context,
            &auto_name_at(url),
            tx,
        );

        let (id, answer, _) = answers
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert_eq!(id, 1);
        assert!(answer.unwrap_err().starts_with("pane read:"));
        assert!(requests
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err());
    }

    /// A catalog entry may rename a workspace by id from anywhere; the pane
    /// the palette opened over is then not one of its panes.
    #[test]
    fn another_workspace_is_described_without_the_focused_panes_output() {
        let (url, requests) = fake_ollama(r#"{"response":"other"}"#, std::time::Duration::ZERO);
        let (tx, answers) = std::sync::mpsc::channel();
        generate(
            2,
            workspace("w2"),
            &stub_herdr("other"),
            &in_a_pane(),
            &auto_name_at(url),
            tx,
        );

        let timeout = std::time::Duration::from_secs(10);
        let (id, answer, _) = answers.recv_timeout(timeout).unwrap();
        assert_eq!((id, answer), (2, Ok("other".to_string())));
        let body: serde_json::Value =
            serde_json::from_str(&requests.recv_timeout(timeout).unwrap()).unwrap();
        assert_eq!(
            body["prompt"],
            "Panes in this workspace:\n- title: w2 pane; cwd: none; git branch: none\n"
        );
    }

    fn workspace(id: &str) -> brief::Subject {
        brief::Subject::Workspace(id.into())
    }

    fn sent_for(name: &str, subject: brief::Subject, context: &Context) -> serde_json::Value {
        let (url, requests) = fake_ollama(r#"{"response":"named"}"#, std::time::Duration::ZERO);
        let (tx, answers) = std::sync::mpsc::channel();
        generate(
            5,
            subject,
            &stub_herdr(name),
            context,
            &auto_name_at(url),
            tx,
        );

        let timeout = std::time::Duration::from_secs(10);
        let (id, answer, _) = answers.recv_timeout(timeout).unwrap();
        assert_eq!((id, answer), (5, Ok("named".to_string())));
        serde_json::from_str(&requests.recv_timeout(timeout).unwrap()).unwrap()
    }

    #[test]
    fn a_tab_is_described_by_its_own_panes_and_the_focused_output() {
        let body = sent_for("tab", brief::Subject::Tab("w1:t1".into()), &in_a_pane());
        assert_eq!(
            body["prompt"],
            "Panes in this tab:\n\
             - title: cargo test; cwd: /nonexistent/a; git branch: none\n\
             - title: vim notes; cwd: none; git branch: none\n\
             \n\
             Last 120 lines of the focused pane:\n\
             last screen line\n"
        );
        assert!(body["system"]
            .as_str()
            .unwrap()
            .starts_with("You name terminal tabs."));
    }

    /// A catalog entry may rename a tab by id from anywhere; the pane the
    /// palette opened over is then not one of its panes.
    #[test]
    fn another_tab_is_described_without_the_focused_panes_output() {
        let body = sent_for(
            "other-tab",
            brief::Subject::Tab("w1:t2".into()),
            &in_a_pane(),
        );
        assert_eq!(
            body["prompt"],
            "Panes in this tab:\n- title: logs; cwd: /nonexistent/b; git branch: none\n"
        );
    }

    fn failure_for(name: &str, subject: brief::Subject) -> String {
        let (url, requests) = fake_ollama("{}", std::time::Duration::ZERO);
        let (tx, answers) = std::sync::mpsc::channel();
        generate(
            1,
            subject,
            &stub_herdr(name),
            &in_a_pane(),
            &auto_name_at(url),
            tx,
        );

        let (_, answer, _) = answers
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert!(requests
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err());
        answer.unwrap_err()
    }

    #[test]
    fn a_tab_with_no_panes_is_not_found_rather_than_named_from_nothing() {
        let err = failure_for("no-tab", brief::Subject::Tab("w1:t9".into()));
        assert_eq!(err, "pane list: tab w1:t9 not found");
    }

    #[test]
    fn a_pane_herdr_cannot_get_names_the_call_that_failed() {
        let err = failure_for("no-pane", brief::Subject::Pane("w1:p9".into()));
        assert!(err.starts_with("pane get: "), "{err}");
    }

    #[test]
    fn a_pane_is_described_by_itself_and_its_own_output() {
        let body = sent_for("pane", brief::Subject::Pane("w1:p3".into()), &in_a_pane());
        assert_eq!(
            body["prompt"],
            "This pane:\n\
             - title: logs; cwd: /nonexistent/b; git branch: none\n\
             \n\
             Last 120 lines of this pane:\n\
             tail -f server.log\n"
        );
        assert!(body["system"]
            .as_str()
            .unwrap()
            .starts_with("You name terminal panes."));
    }

    /// Both refusals are decided before anything is spawned, so the bin path
    /// is never reached.
    #[test]
    fn a_worktree_listing_with_nothing_to_scope_or_fill_is_refused() {
        let herdr = Herdr::new("/nonexistent-herdr-binary".to_string());
        let mut open = command("worktree.open", &["worktree", "open", "--path", "{}"], &[]);
        open.resolve = Some("worktree list".into());

        let err = targets_for(&herdr, &open, &Context::default()).expect_err("no workspace");
        assert!(err.contains("no workspace"), "{err}");

        let mut branch = open.clone();
        branch.args[2] = "--branch".into();
        let context = Context {
            workspace_id: Some("w1".into()),
            ..Context::default()
        };
        let err = targets_for(&herdr, &branch, &context).expect_err("no column");
        assert!(err.contains("--path"), "{err}");
    }

    fn command(id: &str, args: &[&str], contexts: &[&str]) -> catalog::Command {
        catalog::Command {
            id: id.into(),
            title: id.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            contexts: contexts.iter().map(|s| s.to_string()).collect(),
            resolve: None,
            prompt: None,
            binding: None,
            retired_icon: None,
        }
    }

    fn in_a_pane() -> Context {
        Context {
            focused_pane_id: Some("w1:p1".into()),
            tab_id: Some("w1:t1".into()),
            workspace_id: Some("w1".into()),
        }
    }

    #[test]
    fn a_command_is_spawned_with_the_argv_it_carries() {
        let (id, args) = argv(Outcome::Command {
            id: "tab.create".into(),
            args: vec!["tab".into(), "create".into()],
        });
        assert_eq!(id, "tab.create");
        assert_eq!(args, ["tab", "create"]);
    }

    /// Order is herdr's, not ours: the action id is the positional and `--plugin`
    /// follows it — put first, herdr rejects it with `unknown option` (#80).
    #[test]
    fn an_action_is_spawned_as_plugin_action_invoke() {
        let (id, args) = argv(Outcome::Action {
            id: "notes.capture".into(),
            plugin_id: "notes".into(),
            action_id: "capture".into(),
        });
        assert_eq!(id, "notes.capture");
        assert_eq!(
            args,
            ["plugin", "action", "invoke", "capture", "--plugin", "notes"]
        );
    }
}
