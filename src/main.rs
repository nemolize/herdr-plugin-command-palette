//! Pane entrypoint (docs/design.md §3). All rendering, input, and dispatch live
//! here; the action hop only opens the pane.
mod app;
mod catalog;
mod context;
mod frecency;
mod fuzzy;
mod herdr;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

use app::{App, Candidate, Outcome, Step};
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

fn run() -> Result<(), String> {
    let bin = std::env::var("HERDR_BIN_PATH")
        .map_err(|_| "HERDR_BIN_PATH is not set — this runs as a herdr plugin pane".to_string())?;
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

    // Defaulted rather than propagated because a palette of built-ins is still
    // a working palette (§4).
    let actions = herdr.plugin_actions().unwrap_or_default();
    let candidates = assemble(
        catalog.commands,
        actions,
        &context,
        plugin_id.as_deref(),
        herdr::current_platform(),
    );

    if candidates.is_empty() {
        return Err(format!(
            "no commands available (catalog: {})",
            catalog_path.display()
        ));
    }

    let frecency_path = state_dir.as_deref().map(frecency::path);
    let frecency = frecency_path
        .as_deref()
        .map(Frecency::load)
        .unwrap_or_default();

    let mut app = App::new(candidates, frecency);

    // The catalog drifts when Herdr changes its CLI and nothing detects that
    // automatically (§4). A running Herdr older than the version the catalog was
    // checked against is the one case that IS detectable, so it is surfaced —
    // as a footer note rather than a refusal, because most entries still work.
    if let (Some(required), Some(actual)) = (checked_against.as_deref(), herdr.version()) {
        if catalog::is_older(&actual, required) == Some(true) {
            app.status = Some(format!(
                "herdr {actual} is older than the catalog's {required} — some entries may fail"
            ));
        }
    }

    let mut screen = ui::Screen::enter()?;

    loop {
        screen.draw(&mut app)?;
        match ui::next_step(&mut app)? {
            Step::Continue => {}
            Step::Cancel => return Ok(()),
            Step::NeedsTargets(command) => {
                let resolve = command.resolve.clone().unwrap_or_default();
                match herdr.targets(&resolve) {
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
                    Dispatched::Failed(screen, e) => {
                        app.status = Some(e);
                        screen
                    }
                };
            }
        }
    }
}

/// Everything the palette offers, from the two sources that feed it. What each
/// filter drops is invisible once the list is drawn — an entry that should have
/// been excluded looks exactly like one the user has not scrolled to.
fn assemble(
    commands: Vec<catalog::Command>,
    actions: Vec<herdr::PluginAction>,
    context: &Context,
    own_plugin_id: Option<&str>,
    platform: &str,
) -> Vec<Candidate> {
    let scope = context.scope();

    let mut candidates: Vec<Candidate> = commands
        .into_iter()
        .filter(|c| c.available_in(scope) && context.can_satisfy(&c.args))
        .map(|mut c| {
            c.args = context.substitute(&c.args);
            Candidate::from_command(c)
        })
        .collect();

    candidates.extend(
        actions
            .into_iter()
            // Excluded because our own `open` is what launched this palette, so
            // offering it inside itself only reaches `popup already open`.
            .filter(|a| Some(a.plugin_id.as_str()) != own_plugin_id)
            .filter(|a| a.runs_on(platform))
            .filter(|a| a.contexts.is_empty() || a.contexts.iter().any(|c| c == scope))
            .map(Candidate::from_action),
    );

    candidates
}

enum Dispatched {
    Ran,
    Failed(ui::Screen, String),
}

/// Drops the screen, runs `f`, and re-enters only to carry a failure back to
/// the user — success is the palette's exit, so restoring it there would flash
/// the popup back up after the command it was closed for.
fn run_without_screen<F>(screen: ui::Screen, f: F) -> Result<Dispatched, String>
where
    F: FnOnce() -> Result<(), String>,
{
    drop(screen);
    let Err(failure) = f() else {
        return Ok(Dispatched::Ran);
    };
    // A screen that will not reopen has nowhere to show the failure, so it
    // leaves as this function's own error and `main` prints it.
    let screen = ui::Screen::enter().map_err(|_| failure.clone())?;
    Ok(Dispatched::Failed(screen, failure))
}

/// Runs what the user picked. A failure names the command id, so a drifted
/// catalog entry reports itself the first time it is used instead of silently
/// doing nothing (§4).
fn dispatch(herdr: &Herdr, outcome: Outcome) -> Result<(), String> {
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
                "plugin", "action", "invoke", "--plugin", &plugin_id, &action_id,
            ]
            .map(str::to_owned)
            .to_vec(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(id: &str, args: &[&str], contexts: &[&str]) -> catalog::Command {
        catalog::Command {
            id: id.into(),
            title: id.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            contexts: contexts.iter().map(|s| s.to_string()).collect(),
            resolve: None,
        }
    }

    fn action(
        plugin: &str,
        id: &str,
        contexts: &[&str],
        platforms: &[&str],
    ) -> herdr::PluginAction {
        herdr::PluginAction {
            plugin_id: plugin.into(),
            action_id: id.into(),
            title: format!("{plugin}.{id}"),
            contexts: contexts.iter().map(|s| s.to_string()).collect(),
            platforms: platforms.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn in_a_pane() -> Context {
        Context {
            focused_pane_id: Some("w1:p1".into()),
            tab_id: Some("w1:t1".into()),
            workspace_id: Some("w1".into()),
        }
    }

    fn ids(candidates: &[Candidate]) -> Vec<&str> {
        candidates.iter().map(|c| c.id.as_str()).collect()
    }

    /// Offering our own opener inside the palette it opened reaches only
    /// `popup already open`, and nothing downstream would report that as wrong.
    #[test]
    fn the_palettes_own_action_is_not_offered_inside_itself() {
        let candidates = assemble(
            Vec::new(),
            vec![
                action("command-palette", "open", &[], &[]),
                action("notes", "capture", &[], &[]),
            ],
            &in_a_pane(),
            Some("command-palette"),
            "linux",
        );
        assert_eq!(ids(&candidates), ["notes.capture"]);
    }

    /// Without `HERDR_PLUGIN_ID` there is no id to compare against, so the
    /// filter cannot fire — every action stands, including ours.
    #[test]
    fn an_unknown_own_id_excludes_nothing() {
        let candidates = assemble(
            Vec::new(),
            vec![action("command-palette", "open", &[], &[])],
            &in_a_pane(),
            None,
            "linux",
        );
        assert_eq!(ids(&candidates), ["command-palette.open"]);
    }

    /// The filters compose: an entry has to clear all of them, and each one
    /// dropped here is dropped for exactly one reason, named in its id.
    #[test]
    fn an_entry_must_clear_every_filter_to_be_offered() {
        let candidates = assemble(
            vec![
                command("pane.split", &["pane", "split", "{pane}"], &["pane"]),
                command(
                    "needs.absent.workspace.id",
                    &["workspace", "close", "{workspace}"],
                    &[],
                ),
                command("wrong.context", &["tab", "create"], &["tab"]),
            ],
            vec![
                action("other", "wrong-platform", &[], &["windows"]),
                action("other", "wrong-context", &["workspace"], &[]),
                action("other", "offered", &["pane"], &["linux"]),
            ],
            &Context {
                focused_pane_id: Some("w1:p1".into()),
                tab_id: None,
                workspace_id: None,
            },
            Some("command-palette"),
            "linux",
        );
        assert_eq!(ids(&candidates), ["pane.split", "other.offered"]);
    }

    /// The ids the invocation carries are substituted at assembly time, so what
    /// reaches `dispatch` is already the argv herdr runs.
    #[test]
    fn context_ids_are_substituted_into_the_argv() {
        let candidates = assemble(
            vec![command(
                "pane.split",
                &["pane", "split", "--pane", "{pane}"],
                &["pane"],
            )],
            Vec::new(),
            &in_a_pane(),
            None,
            "linux",
        );
        match &candidates[0].kind {
            app::Kind::Command(c) => {
                assert_eq!(c.args, ["pane", "split", "--pane", "w1:p1"]);
            }
            _ => panic!("expected a catalog command"),
        }
    }

    /// A herdr that cannot list actions leaves the built-ins, which is why the
    /// caller defaults the error away rather than propagating it.
    #[test]
    fn built_ins_stand_alone_when_no_actions_are_listed() {
        let candidates = assemble(
            vec![command("tab.create", &["tab", "create"], &[])],
            Vec::new(),
            &in_a_pane(),
            Some("command-palette"),
            "linux",
        );
        assert_eq!(ids(&candidates), ["tab.create"]);
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

    /// The flag name and the operand order are herdr's, not ours: `--plugin`
    /// takes the plugin and the action id follows as a positional.
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
            ["plugin", "action", "invoke", "--plugin", "notes", "capture"]
        );
    }
}
