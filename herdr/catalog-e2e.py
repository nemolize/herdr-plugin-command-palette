#!/usr/bin/env python3
"""Run every catalog entry against a real herdr and fail on any that it rejects.

The unit tests in `src/catalog.rs` check entries against a hand-copied
transcription of `herdr <sub> <cmd> --help`, so they go stale the moment herdr
adds a constraint the table does not carry — which is how `pane.move.tab`
shipped with `--tab` and no `--split` even though its flags were valid and its
positional count was right (docs/design.md §4, issue #24).

This asks herdr instead. herdr's clap layer accepts that combination and its
runtime rejects it, writing usage to stderr and exiting non-zero, so the exit
code alone separates a runnable entry from a broken one.

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


def herdr(*args: str, home: Path, check: bool = True) -> subprocess.CompletedProcess:
    env = dict(os.environ, XDG_CONFIG_HOME=str(home))
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
    env = dict(os.environ, XDG_CONFIG_HOME=str(home))
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


class Fixture:
    """A session holding two tabs, so an entry that needs a second target has one.

    `pane.move.tab` moves a pane into a *different* tab, and `tab.focus` is only
    meaningful with somewhere to switch to.
    """

    def __init__(self, home: Path):
        self.home = home
        self.server = start_server_and_await_readiness(home)
        herdr("workspace", "create", home=home)
        herdr("tab", "create", home=home)
        self.ids = self._read_ids()

    def _read_ids(self) -> dict[str, str]:
        panes = json.loads(herdr("pane", "list", home=self.home).stdout)
        rows = panes["result"]["panes"]
        focused = next((p for p in rows if p.get("focused")), rows[0])
        tabs = {p["tab_id"] for p in rows}
        another_tab = next((t for t in sorted(tabs) if t != focused["tab_id"]), None)
        return {
            "{pane}": focused["pane_id"],
            "{tab}": focused["tab_id"],
            "{workspace}": focused["workspace_id"],
            "{}": another_tab or focused["tab_id"],
        }

    def close(self) -> None:
        herdr("server", "stop", home=self.home, check=False)
        try:
            self.server.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.server.kill()


def resolved_args(entry: dict, ids: dict[str, str]) -> list[str]:
    """Substitute the placeholders the palette would have filled at open time.

    `{pane}` / `{tab}` / `{workspace}` come from the invocation's context
    (`src/context.rs`); `{}` is the row the user picks from the list named by
    `resolve` (`src/catalog.rs`), so which list that is decides what `{}` holds.
    A tab id is the default because most resolving entries take one.
    """
    table = dict(ids)
    resolve = entry.get("resolve")
    if resolve == "workspace list":
        table["{}"] = ids["{workspace}"]
    elif resolve == "pane list":
        table["{}"] = ids["{pane}"]

    return [table.get(a, a) for a in entry["args"]]


def note_if_pin_disagrees_with_running_herdr(checked_against: str | None) -> None:
    """Say so when the catalog's pin names a different herdr than the one that ran.

    Not an error — the entries either run here or they do not, and that verdict
    stands either way. But a green run against a herdr the catalog was never
    checked on leaves `checked_against` behind what has actually been verified,
    and nothing else in the run would say so.
    """
    proc = subprocess.run([HERDR, "--version"], capture_output=True, text=True)
    running = proc.stdout.split()[-1] if proc.returncode == 0 and proc.stdout else None
    if checked_against and running and running != checked_against:
        print(
            f"note: catalog says checked_against = {checked_against}, "
            f"running herdr {running}",
            file=sys.stderr,
        )


def report_failures(failures: list[tuple[str, list[str], int, str]]) -> None:
    """Print the entry id and argv per failure, because #24's whole point is that
    a drifted catalog must say which line to edit — herdr's own runtime already
    reports that something is wrong and that was not enough."""
    plural = "y" if len(failures) == 1 else "ies"
    print(f"\n{len(failures)} catalog entr{plural} herdr rejected:\n", file=sys.stderr)
    for entry_id, args, code, stderr in failures:
        print(f"  {entry_id}", file=sys.stderr)
        print(f"    argv: herdr {' '.join(args)}", file=sys.stderr)
        print(f"    exit: {code}", file=sys.stderr)
        for line in stderr.splitlines()[:4]:
            print(f"    {line}", file=sys.stderr)
        print(file=sys.stderr)


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

    note_if_pin_disagrees_with_running_herdr(catalog.get("checked_against"))

    FIXTURE_ROOT.mkdir(parents=True, exist_ok=True)
    failures: list[tuple[str, list[str], int, str]] = []

    for entry in entries:
        home = Path(tempfile.mkdtemp(dir=FIXTURE_ROOT))
        try:
            fixture = Fixture(home)
        except RuntimeError as e:
            print(f"FAIL {entry['id']}: {e}", file=sys.stderr)
            shutil.rmtree(home, ignore_errors=True)
            return 1

        try:
            args = resolved_args(entry, fixture.ids)
            proc = herdr(*args, home=home, check=False)
            if proc.returncode != 0:
                failures.append((entry["id"], args, proc.returncode, proc.stderr.strip()))
                print(f"FAIL {entry['id']}", file=sys.stderr)
            else:
                print(f"ok   {entry['id']}", file=sys.stderr)
        finally:
            fixture.close()
            shutil.rmtree(home, ignore_errors=True)

    if failures:
        report_failures(failures)
        return 1

    print(
        f"\nall {len(entries)} catalog entries ran against herdr "
        f"{catalog.get('checked_against', '?')}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
