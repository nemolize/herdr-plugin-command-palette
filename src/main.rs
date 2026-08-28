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

    let min_herdr_version = catalog.min_herdr_version.clone();

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
    if let (Some(required), Some(actual)) = (min_herdr_version.as_deref(), herdr.version()) {
        if catalog::is_older(&actual, required) == Some(true) {
            app.status = Some(format!(
                "herdr {actual} is older than the catalog's {required} — some entries may fail"
            ));
        }
    }

    let mut screen = ui::Screen::enter()?;

    let outcome = loop {
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
            Step::Run(outcome) => break outcome,
        }
    };

    // Persist the ranking before the screen is torn down, so a dispatch failure
    // below still leaves the ordering updated — the user did pick it.
    if let Some(path) = frecency_path.as_deref() {
        let _ = app.frecency().save(path);
    }
    drop(screen);

    // A failed dispatch names the command id, so a drifted catalog entry reports
    // itself the first time it is used instead of silently doing nothing (§4).
    match outcome {
        Outcome::Command { id, args } => herdr
            .dispatch(&args)
            .map_err(|e| format!("`{id}` failed: {e}")),
        Outcome::Action {
            id,
            plugin_id,
            action_id,
        } => {
            let args = [
                "plugin", "action", "invoke", "--plugin", &plugin_id, &action_id,
            ]
            .map(str::to_owned)
            .to_vec();
            herdr
                .dispatch(&args)
                .map_err(|e| format!("`{id}` failed: {e}"))
        }
    }
}
