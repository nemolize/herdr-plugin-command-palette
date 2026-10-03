#!/usr/bin/env python3
"""Run every catalog entry against a real herdr and fail on any that it rejects.

The unit tests in `src/catalog.rs` check entries against a hand-copied
transcription of `herdr <sub> <cmd> --help`, so they go stale the moment herdr
adds a constraint the table does not carry — which is how `pane.move.tab`
shipped with `--tab` and no `--split` even though its flags were valid and its
positional count was right (docs/design.md §4, issue #24).

This asks herdr instead. herdr's clap layer accepts that combination and its
runtime rejects it, writing usage to stderr and exiting non-zero, so the exit
code separates a runnable entry from a broken one. An entry that exits 0 must
also answer with an envelope the palette accepts — a non-null `result` and no
`error` — because the palette reports anything else as a failed action (#146).

Every entry gets its own fixture session, built from nothing and torn down
after. That is what makes running the destructive entries (`pane close`,
`workspace close`) safe, and it removes the ordering coupling that would
otherwise decide whether an entry finds the state it needs.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CATALOG = REPO / "herdr" / "catalog.toml"

# Short because the api socket lands under it, and a unix socket path over
# ~104 bytes fails with `sun_path capacity` — which reads as a herdr fault.
FIXTURE_ROOT = Path(os.environ.get("HERDR_E2E_ROOT", "/tmp/herdr-e2e"))

SESSION = "catalog-e2e"
BOOT_TIMEOUT_SECS = 30
BOOT_POLL_SECS = 0.2


def herdr_binary() -> str | None:
    """Prefer the one `just fetch-herdr` puts in ./bin over whatever is on PATH,
    so a developer's own herdr is not what CI's verdict silently rests on."""
    fetched = REPO / "bin" / "herdr"
    if fetched.is_file() and os.access(fetched, os.X_OK):
        return str(fetched)
    return shutil.which("herdr")


HERDR = herdr_binary()


def fixture_env(home: Path) -> dict[str, str]:
    """HOME as well as the config dir: herdr creates worktrees under
    `~/.herdr/worktrees`, so a real HOME would collect one per run."""
    return dict(os.environ, XDG_CONFIG_HOME=str(home), HOME=str(home))


def herdr(*args: str, home: Path, check: bool = True) -> subprocess.CompletedProcess:
    env = fixture_env(home)
    proc = subprocess.run(
        [HERDR, "--session", SESSION, *args],
        env=env,
        capture_output=True,
        text=True,
    )
    if check and proc.returncode != 0:
        raise RuntimeError(
            f"fixture setup failed: herdr {' '.join(args)}\n"
            f"exit={proc.returncode}\n{proc.stderr.strip()}"
        )
    return proc


def start_server_and_await_readiness(home: Path) -> subprocess.Popen:
    env = fixture_env(home)
    log = (home / "server.log").open("w")
    server = subprocess.Popen(
        [HERDR, "--session", SESSION, "server"],
        env=env,
        stdout=log,
        stderr=subprocess.STDOUT,
    )

    deadline = time.monotonic() + BOOT_TIMEOUT_SECS
    while time.monotonic() < deadline:
        if server.poll() is not None:
            raise RuntimeError(
                f"herdr server exited during boot (code {server.returncode})\n"
                f"{(home / 'server.log').read_text()[:2000]}"
            )
        if herdr("pane", "list", home=home, check=False).returncode == 0:
            return server
        time.sleep(BOOT_POLL_SECS)

    server.kill()
    raise RuntimeError(
        f"herdr server did not answer within {BOOT_TIMEOUT_SECS}s\n"
        f"{(home / 'server.log').read_text()[:2000]}"
    )


def init_repository(path: Path, home: Path) -> None:
    """A one-commit repository: `worktree create` needs a HEAD to branch from."""
    path.mkdir()
    for args in (
        ["init", "--quiet"],
        ["-c", "user.name=e2e", "-c", "user.email=e2e@example.com",
         "commit", "--quiet", "--allow-empty", "--message", "e2e"],
    ):
        proc = subprocess.run(
            ["git", "-C", str(path), *args],
            env=fixture_env(home),
            capture_output=True,
            text=True,
        )
        if proc.returncode != 0:
            raise RuntimeError(
                f"fixture setup failed: git {' '.join(args)}\n"
                f"exit={proc.returncode}\n{proc.stderr.strip()}"
            )


class Fixture:
    """A session holding two tabs, so an entry that needs a second target has one.

    `pane.move.tab` moves a pane into a *different* tab, and `tab.focus` is only
    meaningful with somewhere to switch to. The workspace is rooted in a Git
    repository so the worktree entries have one to act on, and `worktree` adds
    a linked worktree — `remove` refuses the main checkout, so acting on that
    one would prove nothing. `closed` closes its workspace, so `open` opens it
    rather than focusing what is already there.
    """

    def __init__(self, home: Path, worktree: bool = False, closed: bool = False):
        self.home = home
        repo = home / "repo"
        init_repository(repo, home)
        self.server = start_server_and_await_readiness(home)
        # Past this point the server is running, so anything that raises has to
        # stop it — an exception here binds no Fixture for the caller to close.
        try:
            herdr("workspace", "create", "--cwd", str(repo), home=home)
            herdr("tab", "create", home=home)
            self.ids = self._read_ids()
            if worktree:
                self.ids.update(self._create_worktree(closed))
        except BaseException:
            self.close()
            raise

    def _create_worktree(self, closed: bool) -> dict[str, str]:
        proc = herdr(
            "worktree", "create", "--workspace", self.ids["{workspace}"],
            "--branch", "e2e-existing", "--no-focus", home=self.home,
        )
        row = json.loads(proc.stdout)["result"]["worktree"]
        # The linked worktree's workspace is where `create` and `open` refuse to
        # start from, so the context is moved there while it stays open.
        listing = herdr(
            "worktree", "list", "--workspace", row["open_workspace_id"], home=self.home
        )
        repo = json.loads(listing.stdout)["result"]["source"]["repo_root"]
        ids = {}
        if closed:
            herdr("workspace", "close", row["open_workspace_id"], home=self.home)
        else:
            ids["{workspace}"] = row["open_workspace_id"]
        return {
            **ids,
            "{repo}": repo,
            "{worktree path}": row["path"],
            "{worktree workspace}": row["open_workspace_id"],
        }

    def _read_ids(self) -> dict[str, str]:
        panes = json.loads(herdr("pane", "list", home=self.home).stdout)
        rows = panes["result"]["panes"]
        focused = next((p for p in rows if p.get("focused")), rows[0])
        tabs = {p["tab_id"] for p in rows}
        another_tab = next((t for t in sorted(tabs) if t != focused["tab_id"]), None)
        if another_tab is None:
            # Falling back to the focused tab would let `pane.move.tab` move a
            # pane into the tab it already occupies and still report ok.
            raise RuntimeError(
                f"fixture has only one tab ({focused['tab_id']}); "
                "the entries needing a second target would not be exercised"
            )
        return {
            "{pane}": focused["pane_id"],
            "{tab}": focused["tab_id"],
            "{workspace}": focused["workspace_id"],
            "{another tab}": another_tab,
        }

    def close(self) -> None:
        herdr("server", "stop", home=self.home, check=False)
        try:
            self.server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.server.kill()


def flag_before(args: list[str], placeholder: str) -> str | None:
    at = args.index(placeholder) if placeholder in args else 0
    return args[at - 1] if at > 0 else None


def resolved_args(entry: dict, ids: dict[str, str]) -> list[str]:
    """Substitute the placeholders the palette would have filled at open time.

    A tab-resolving entry takes the tab the pane is NOT in, so the move it
    performs is a real one. `{text}` carries a space because that is the shape
    that breaks if the palette ever splits the typed name across argv elements —
    except as a branch name, which git refuses with one.

    Raises on a `resolve` this file does not know: defaulting it to a tab id
    would substitute a plausible argument into an entry never taught here and
    report ok, which is the silent staleness this check exists to remove.
    """
    args = entry["args"]
    table = dict(ids)
    table["{text}"] = "e2e-created" if flag_before(args, "{text}") == "--branch" else "e2e renamed"
    resolve = entry.get("resolve")
    if resolve is not None:
        picked = {
            "tab list": "{another tab}",
            "pane list": "{pane}",
            "workspace list": "{workspace}",
            "worktree list": {
                "--path": "{worktree path}",
                "--workspace": "{worktree workspace}",
            }.get(flag_before(args, "{}")),
        }.get(resolve)
        if picked is None:
            raise RuntimeError(
                f"{entry['id']}: resolve = {resolve!r} is not a list this "
                "harness knows how to pick a target from"
            )
        table["{}"] = ids[picked]

    return [table.get(a, a) for a in args]


def parse_envelope(text: str) -> dict | None:
    try:
        body = json.loads(text)
    except ValueError:
        return None
    return body if isinstance(body, dict) else None


def unaccepted_response(stdout: str, stderr: str) -> str | None:
    """Why `read_response` in src/herdr.rs would call this answer a failure, or
    None when it would accept it. Mirrors its order: the envelope from stdout,
    else from stderr; then `error`; then a `result` that is absent or null."""
    body = parse_envelope(stdout)
    if body is None:
        body = parse_envelope(stderr)
    if body is None:
        return "no JSON envelope"
    if body.get("error") is not None:
        return f"an error: {json.dumps(body['error'])}"
    if "result" not in body:
        return "no result"
    if body["result"] is None:
        return "a null result"
    return None


def running_herdr_version() -> str | None:
    proc = subprocess.run([HERDR, "--version"], capture_output=True, text=True)
    words = proc.stdout.split() if proc.returncode == 0 else []
    return words[-1] if words else None


def note_if_pin_disagrees_with_running_herdr(
    checked_against: str | None, running: str | None
) -> None:
    """Say so when the catalog's pin names a different herdr than the one that ran.

    Not an error — the entries either run here or they do not, and that verdict
    stands either way. It prints before the entries run, so a red run reports
    the mismatch too, where the success line never prints.
    """
    if checked_against and running and running != checked_against:
        print(
            f"note: catalog says checked_against = {checked_against}, "
            f"running herdr {running}",
            file=sys.stderr,
        )


def success_line(count: int, checked_against: str | None, running: str | None) -> str:
    line = f"all {count} catalog entries ran against herdr {running or '(version unknown)'}"
    if checked_against and running != checked_against:
        line += f" (catalog checked_against = {checked_against})"
    return line


def report_rejected(rejected: list[tuple[str, list[str], int, str]]) -> None:
    """Print the entry id and argv per rejection, because #24's whole point is
    that a drifted catalog must say which line to edit — herdr's own runtime
    already reports that something is wrong and that was not enough."""
    plural = "y" if len(rejected) == 1 else "ies"
    print(f"\n{len(rejected)} catalog entr{plural} herdr rejected:\n", file=sys.stderr)
    for entry_id, args, code, stderr in rejected:
        print(f"  {entry_id}", file=sys.stderr)
        print(f"    argv: herdr {' '.join(args)}", file=sys.stderr)
        print(f"    exit: {code}", file=sys.stderr)
        for line in stderr.splitlines()[:4]:
            print(f"    {line}", file=sys.stderr)
        print(file=sys.stderr)


def report_unaccepted(unaccepted: list[tuple[str, list[str], str, str, str]]) -> None:
    """Report separately from rejections: herdr ran these and exited 0, so the
    argv is fine — it is the answer that changed, and the palette would show a
    failure for an action that worked."""
    plural = "y" if len(unaccepted) == 1 else "ies"
    print(
        f"\n{len(unaccepted)} catalog entr{plural} herdr ran but answered with "
        "a response the palette reports as a failure:\n",
        file=sys.stderr,
    )
    for entry_id, args, reason, stdout, stderr in unaccepted:
        print(f"  {entry_id}", file=sys.stderr)
        print(f"    argv: herdr {' '.join(args)}", file=sys.stderr)
        print(f"    answered with {reason}", file=sys.stderr)
        for name, text in (("stdout", stdout), ("stderr", stderr)):
            for line in text.splitlines()[:4]:
                print(f"    {name}: {line}", file=sys.stderr)
        print(file=sys.stderr)


def report_broken(broken: list[tuple[str, str]]) -> None:
    """Report separately from rejections: this is the harness failing to build a
    session, not the catalog being wrong, and reading one as the other sends
    whoever is on the red build to edit a file that is fine."""
    plural = "y" if len(broken) == 1 else "ies"
    print(
        f"\n{len(broken)} entr{plural} could not be checked — the fixture "
        "failed to build, which is a harness fault rather than catalog drift:\n",
        file=sys.stderr,
    )
    for entry_id, message in broken:
        print(f"  {entry_id}", file=sys.stderr)
        for line in message.splitlines()[:6]:
            print(f"    {line}", file=sys.stderr)
        print(file=sys.stderr)


def report_unknown_bindings(entries: list[dict], home: Path) -> list[tuple[str, str]]:
    """Every `binding` herdr does not recognise as a `[keys]` action.

    The oracle is `herdr config check` rather than `--default-config`, because
    the template under-reports: it omits `swap_pane_*`, which herdr binds by
    default, so a template-based check calls a working action unknown. Writing
    every binding into one config and reading the warnings asks herdr itself.

    A wrong `binding` is otherwise invisible — it shows a blank key column,
    which is also what a correct entry with no counterpart shows.
    """
    named = [(e["id"], e["binding"]) for e in entries if e.get("binding")]
    if not named:
        return []

    # The verdict is an ABSENCE of warnings, which a crashed or reworded herdr
    # produces too — so a name it cannot know must come back flagged.
    sentinel = "zzz_not_a_herdr_action"
    config = home / "herdr" / "config.toml"
    config.parent.mkdir(parents=True, exist_ok=True)
    lines = "\n".join(f'{action} = "prefix+z"' for _, action in named)
    config.write_text(f'onboarding = false\n[keys]\n{lines}\n{sentinel} = "prefix+z"\n')

    proc = herdr("config", "check", home=home, check=False)
    warned = proc.stdout + proc.stderr

    def flagged(action: str) -> bool:
        return f"keys.{action};" in warned or f"keys.{action} " in warned

    if not flagged(sentinel):
        raise RuntimeError(
            "`herdr config check` did not flag the sentinel "
            f"`{sentinel}`, so its silence proves nothing about the real "
            f"bindings. Output was:\n{warned.strip() or '(nothing)'}"
        )

    return [(entry_id, action) for entry_id, action in named if flagged(action)]


def main() -> int:
    if HERDR is None:
        print(
            "no herdr found — run `just fetch-herdr`, or put one on PATH",
            file=sys.stderr,
        )
        return 1

    catalog = tomllib.loads(CATALOG.read_text())
    entries = catalog.get("command", [])
    if not entries:
        print(f"no entries in {CATALOG}", file=sys.stderr)
        return 1

    checked_against = catalog.get("checked_against")
    running = running_herdr_version()
    note_if_pin_disagrees_with_running_herdr(checked_against, running)

    FIXTURE_ROOT.mkdir(parents=True, exist_ok=True)
    rejected: list[tuple[str, list[str], int, str]] = []
    unaccepted: list[tuple[str, list[str], str, str, str]] = []
    broken: list[tuple[str, str]] = []

    bindings_home = Path(tempfile.mkdtemp(dir=FIXTURE_ROOT))
    try:
        unknown_bindings = report_unknown_bindings(entries, bindings_home)
    except RuntimeError as e:
        print(f"binding check could not run: {e}", file=sys.stderr)
        return 1
    finally:
        shutil.rmtree(bindings_home, ignore_errors=True)
    for entry_id, action in unknown_bindings:
        print(f"FAIL {entry_id}: binding `{action}`", file=sys.stderr)

    for entry in entries:
        home = Path(tempfile.mkdtemp(dir=FIXTURE_ROOT))
        try:
            fixture = Fixture(
                home,
                worktree=entry["args"][0] == "worktree",
                closed=flag_before(entry["args"], "{}") == "--path",
            )
        except RuntimeError as e:
            # Keep going: one flaky boot must not swallow every later entry's
            # verdict, and a harness fault is not a drifted catalog.
            broken.append((entry["id"], str(e)))
            print(f"BROKE {entry['id']}", file=sys.stderr)
            shutil.rmtree(home, ignore_errors=True)
            continue

        try:
            args = resolved_args(entry, fixture.ids)
            proc = herdr(*args, home=home, check=False)
            if proc.returncode != 0:
                rejected.append((entry["id"], args, proc.returncode, proc.stderr.strip()))
                print(f"FAIL {entry['id']}", file=sys.stderr)
            elif (reason := unaccepted_response(proc.stdout, proc.stderr)) is not None:
                unaccepted.append(
                    (entry["id"], args, reason, proc.stdout.strip(), proc.stderr.strip())
                )
                print(f"FAIL {entry['id']}", file=sys.stderr)
            else:
                print(f"ok   {entry['id']}", file=sys.stderr)
        except RuntimeError as e:
            broken.append((entry["id"], str(e)))
            print(f"BROKE {entry['id']}", file=sys.stderr)
        finally:
            fixture.close()
            shutil.rmtree(home, ignore_errors=True)

    if rejected:
        report_rejected(rejected)
    if unaccepted:
        report_unaccepted(unaccepted)
    if broken:
        report_broken(broken)
    if unknown_bindings:
        print(
            f"\n{len(unknown_bindings)} catalog `binding` value(s) herdr does not "
            "know as a [keys] action — each shows a blank key column:\n",
            file=sys.stderr,
        )
        for entry_id, action in unknown_bindings:
            print(f"  {entry_id}: binding = \"{action}\"", file=sys.stderr)
    if rejected or unaccepted or broken or unknown_bindings:
        return 1

    print(f"\n{success_line(len(entries), checked_against, running)}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
