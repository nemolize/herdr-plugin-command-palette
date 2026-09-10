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
    let scope = context.scope();

    let checked_against = catalog.checked_against.clone();

    let mut candidates: Vec<Candidate> = catalog
        .commands
        .into_iter()
        .filter(|c| c.available_in(scope) && context.can_satisfy(&c.args))
        .map(|mut c| {
            c.args = context.substitute(&c.args);
            Candidate::from_command(c)
        })
        .collect();

    // Plugin actions merge into the same list (§4). Their absence is not fatal:
    // a palette of built-ins is still a working palette.
    if let Ok(actions) = herdr.plugin_actions() {
        let platform = herdr::current_platform();
        candidates.extend(
            actions
                .into_iter()
                // Our own `open` is what launched this palette; offering it
                // inside itself only reaches `popup already open`.
                .filter(|a| Some(&a.plugin_id) != plugin_id.as_ref())
                .filter(|a| a.runs_on(platform))
                .filter(|a| a.contexts.is_empty() || a.contexts.iter().any(|c| c == scope))
                .map(Candidate::from_action),
        );
    }

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

/// What a dispatch attempt leaves behind: nothing on success, and on failure
/// the reopened palette plus the message it has to show.
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

    /// A catalog entry is spawned as written — anything inserted or dropped
    /// here reaches herdr as a different command than the palette listed.
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
